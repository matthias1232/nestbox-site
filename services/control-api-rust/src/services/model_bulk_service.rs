//! One model on many nodes as one job (package B04).
//!
//! The call plans first (which nodes, which verdict, which are skipped and why), creates one
//! `ModelBulkInstall` job whose targets are the nodes that will be installed, and answers at
//! once. A detached task then runs the existing per-node install for each target, waits for
//! that install to settle, records the outcome on the job and publishes progress. When the
//! caller asked for a cluster and at least one install succeeded, the task creates it at the
//! end. Nothing here reserves memory itself: `model_service::install` stays the only place
//! that does.

use std::sync::Arc;

use serde::Deserialize;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::job::{JobKind, TargetState};
use crate::domain::model::{Fit, Model};
use crate::domain::model_install::{Artifact, InstallState};
use crate::domain::node::{Node, NodeStatus};
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::jobs as job_store;
use crate::store::postgres::model_installs as install_store;
use crate::store::postgres::models as model_store;
use crate::store::postgres::nodes as node_store;

/// How many node installs run at the same time. The weights come from the public internet, so
/// twenty parallel downloads would saturate the control plane's uplink for no gain.
const PARALLEL_INSTALLS: usize = 4;
/// The shortest time one install may take before the job calls it failed.
const MIN_WAIT_SECONDS: u64 = 600;
/// Assumed download speed used to scale the wait with the size of the weights.
const ASSUMED_BYTES_PER_SECOND: u64 = 20 * 1024 * 1024;
/// Seconds between two looks at an install row.
const POLL_SECONDS: u64 = 5;

/// The body of `POST /models/{key}/install-bulk` (contract C6).
#[derive(Debug, Default, Deserialize)]
struct BulkRequest {
    /// Node names or ids.
    #[serde(default)]
    nodes: Vec<String>,
    /// Every online node of this site.
    #[serde(default)]
    site_id: Option<Uuid>,
    /// Every online node of the data center.
    #[serde(default)]
    environment_wide: bool,
    /// Install even where the fit verdict is only advice against it.
    #[serde(default)]
    force: bool,
    /// Group the successful installs into a cluster at the end.
    #[serde(default)]
    make_cluster: Option<MakeCluster>,
}

/// The cluster to create when the installs are done.
#[derive(Debug, Clone, Deserialize)]
struct MakeCluster {
    name: String,
    #[serde(default = "default_strategy")]
    strategy: String,
}

fn default_strategy() -> String {
    "round_robin".to_string()
}

/// One node of the plan.
#[derive(Debug, Clone)]
struct Target {
    node: Node,
    fit: String,
    skipped: Option<String>,
}

/// Why a verdict keeps a node out of the job, or `None` when the node is a target.
fn skip_reason(fit: Fit) -> Option<&'static str> {
    match fit {
        Fit::AlreadyInstalled => Some("installed"),
        Fit::NoGpu => Some("no_gpu"),
        Fit::NeedsUnifiedMemory => Some("needs_unified_memory"),
        Fit::InsufficientVram
        | Fit::InsufficientGpus
        | Fit::InsufficientRam
        | Fit::InsufficientCpu => Some("too_small"),
        Fit::Remote | Fit::Unknown => Some("not_installable"),
        Fit::Installable | Fit::CpuInstallable | Fit::UpdateAvailable => None,
    }
}

/// The word the answer uses for a verdict: `fits` for the installable ones, else the name.
fn fit_label(fit: Fit) -> String {
    match fit {
        Fit::Installable | Fit::CpuInstallable | Fit::UpdateAvailable => "fits".to_string(),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

/// How long one install may take: ten minutes, or the time the weights need at 20 MiB/s.
fn wait_limit_seconds(weights_bytes: Option<i64>) -> u64 {
    let by_size = weights_bytes
        .and_then(|bytes| u64::try_from(bytes).ok())
        .map_or(0, |bytes| bytes / ASSUMED_BYTES_PER_SECOND);
    by_size.max(MIN_WAIT_SECONDS)
}

/// Plan and start the bulk install of `key`; returns `{job_id, targets}` (contract C6).
pub async fn install_bulk(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ModelInstall)?;
    let request: BulkRequest = serde_json::from_value(body)
        .map_err(|error| ApiError::validation(format!("Invalid request body: {error}")))?;

    let selectors = usize::from(!request.nodes.is_empty())
        + usize::from(request.site_id.is_some())
        + usize::from(request.environment_wide);
    if selectors != 1 {
        return Err(ApiError::validation(
            "Give exactly one of 'nodes', 'site_id' or 'environment_wide'.",
        ));
    }
    if let Some(cluster) = &request.make_cluster {
        if cluster.name.trim().is_empty() {
            return Err(ApiError::validation("The cluster needs a name."));
        }
        if !["round_robin", "least_loaded", "failover"].contains(&cluster.strategy.as_str()) {
            return Err(ApiError::validation(
                "The cluster strategy is round_robin, least_loaded or failover.",
            ));
        }
    }

    let model = load_model(ctx, principal, key).await?;
    if model.artifact_ref.as_deref().and_then(Artifact::parse).is_none()
        || model.serve_runtime.is_none()
    {
        return Err(ApiError::unprocessable(
            ErrorCode::MODEL_NOT_SERVABLE,
            format!(
                "{} has no artefact this platform can fetch, so it cannot be installed as a \
                 local model server.",
                model.name
            ),
        ));
    }

    let nodes = resolve_targets(ctx, principal, &request).await?;
    if nodes.is_empty() {
        return Err(ApiError::validation("No node matches the request."));
    }

    // The verdict per node comes from the same classifier the catalog uses, so the plan and
    // the single install can never disagree about what fits.
    let mut targets: Vec<Target> = Vec::with_capacity(nodes.len());
    for node in nodes {
        if node.status == NodeStatus::Offline {
            targets.push(Target {
                node,
                fit: "unknown".to_string(),
                skipped: Some("offline".to_string()),
            });
            continue;
        }
        let query = super::model_service::CatalogQuery {
            node: Some(node.name.clone()),
            ..Default::default()
        };
        let verdict = match super::model_service::catalog(ctx, principal, false, &query).await {
            Ok(view) => view
                .items
                .into_iter()
                .find(|item| item.model.key == model.key)
                .map(|item| item.fit.fit),
            Err(error) => {
                targets.push(Target {
                    node,
                    fit: "unknown".to_string(),
                    skipped: Some(format!("unreachable: {}", error.message)),
                });
                continue;
            }
        };
        match verdict {
            Some(fit) => targets.push(Target {
                node,
                fit: fit_label(fit),
                skipped: skip_reason(fit).map(str::to_string),
            }),
            None => targets.push(Target {
                node,
                fit: "unknown".to_string(),
                skipped: Some("not_installable".to_string()),
            }),
        }
    }

    let run: Vec<Node> = targets
        .iter()
        .filter(|target| target.skipped.is_none())
        .map(|target| target.node.clone())
        .collect();
    let answer_targets: Vec<serde_json::Value> = targets
        .iter()
        .map(|target| {
            serde_json::json!({
                "node": target.node.name,
                "fit": target.fit,
                "skipped": target.skipped,
            })
        })
        .collect();

    let node_ids: Vec<Uuid> = run.iter().map(|node| node.id).collect();
    let node_names: Vec<&str> = run.iter().map(|node| node.name.as_str()).collect();
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let job = job_store::create(
        &mut scope,
        JobKind::ModelBulkInstall,
        &node_ids,
        &serde_json::json!({
            "model_key": model.key,
            "nodes": node_names,
            "force": request.force,
            "make_cluster": request.make_cluster.as_ref().map(|cluster| serde_json::json!({
                "name": cluster.name,
                "strategy": cluster.strategy,
            })),
        }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    if run.is_empty() {
        job_store::log(&mut scope, job.id, "info", "Every node was skipped; nothing to install.")
            .await
            .map_err(ApiError::from)?;
        job_store::finish(&mut scope, job.id, Some(&serde_json::json!({ "cluster_id": null })))
            .await
            .map_err(ApiError::from)?;
    } else {
        job_store::start(&mut scope, job.id).await.map_err(ApiError::from)?;
    }
    scope.commit().await.map_err(ApiError::from)?;

    if !run.is_empty() {
        let worker = ctx.clone();
        let principal = principal.clone();
        let job_id = job.id;
        let force = request.force;
        let make_cluster = request.make_cluster.clone();
        tokio::spawn(async move {
            if let Err(error) =
                run_job(worker, principal, job_id, model, run, force, make_cluster).await
            {
                tracing::error!(
                    code = error.code.0,
                    job = %job_id,
                    "bulk model install could not record its outcome"
                );
            }
        });
    }

    Ok(serde_json::json!({ "job_id": job.id, "targets": answer_targets }))
}

/// The catalog row for `key`, or 404.
async fn load_model(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
) -> Result<Model, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let model = model_store::get_by_key(&mut scope, key)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::MODEL_NOT_FOUND,
                format!("No model with key '{key}'."),
            ),
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(model)
}

/// The nodes the request names, in a stable order and without duplicates.
async fn resolve_targets(
    ctx: &ServiceContext,
    principal: &Principal,
    request: &BulkRequest,
) -> Result<Vec<Node>, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let visible = node_store::list(&mut scope).await.map_err(ApiError::from)?;
    let mut chosen: Vec<Node> = Vec::new();

    if !request.nodes.is_empty() {
        for reference in &request.nodes {
            let reference = reference.trim();
            let found = visible
                .iter()
                .find(|node| node.name == reference)
                .or_else(|| {
                    Uuid::parse_str(reference)
                        .ok()
                        .and_then(|id| visible.iter().find(|node| node.id == id))
                });
            let Some(node) = found else {
                return Err(ApiError::not_found(
                    ErrorCode::NODE_NOT_FOUND,
                    format!("No node '{reference}'."),
                ));
            };
            if !chosen.iter().any(|existing| existing.id == node.id) {
                chosen.push(node.clone());
            }
        }
    } else if let Some(site_id) = request.site_id {
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM nodes WHERE site_id = $1")
            .bind(site_id)
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        chosen = visible
            .into_iter()
            .filter(|node| ids.contains(&node.id) && node.status == NodeStatus::Online)
            .collect();
    } else {
        chosen = visible
            .into_iter()
            .filter(|node| node.status == NodeStatus::Online)
            .collect();
    }
    scope.commit().await.map_err(ApiError::from)?;
    Ok(chosen)
}

/// What one node's install came to.
struct Outcome {
    node: Node,
    ok: bool,
    install_id: Option<Uuid>,
    message: String,
}

/// The body of the detached job: install on every target, record, then maybe make the cluster.
async fn run_job(
    ctx: ServiceContext,
    principal: Principal,
    job_id: Uuid,
    model: Model,
    nodes: Vec<Node>,
    force: bool,
    make_cluster: Option<MakeCluster>,
) -> Result<(), ApiError> {
    let environment_id = principal.environment_id;
    let limit = wait_limit_seconds(model.weights_bytes);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(PARALLEL_INSTALLS));
    let mut tasks = tokio::task::JoinSet::new();
    for node in nodes {
        let ctx = ctx.clone();
        let principal = principal.clone();
        let key = model.key.clone();
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok();
            install_one(&ctx, &principal, &key, node, force, limit).await
        });
    }

    let mut ok_installs: Vec<Uuid> = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok(outcome) = joined else { continue };
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let state = if outcome.ok { TargetState::Succeeded } else { TargetState::Failed };
        job_store::record_target(
            &mut scope,
            job_id,
            outcome.node.id,
            state,
            if outcome.ok { None } else { Some(outcome.message.as_str()) },
        )
        .await
        .map_err(ApiError::from)?;
        let line = serde_json::json!({
            "node": outcome.node.name,
            "ok": outcome.ok,
            "install_id": outcome.install_id,
            "message": outcome.message,
        });
        job_store::log(
            &mut scope,
            job_id,
            if outcome.ok { "info" } else { "error" },
            &line.to_string(),
        )
        .await
        .map_err(ApiError::from)?;
        let job = job_store::get(&mut scope, job_id).await.map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        ctx.bus.publish(LiveEvent::JobProgress {
            job_id,
            environment_id,
            completed: job.completed + job.failed,
            total: job.total,
            status: "running".to_string(),
        });
        if let (true, Some(id)) = (outcome.ok, outcome.install_id) {
            ok_installs.push(id);
        }
    }

    let mut result = serde_json::json!({ "cluster_id": null });
    if let Some(cluster) = make_cluster {
        if ok_installs.is_empty() {
            result["cluster_error"] = "No install succeeded, so no cluster was made.".into();
        } else {
            let body = serde_json::json!({
                "name": cluster.name,
                "model_key": model.key,
                "strategy": cluster.strategy,
                "install_ids": ok_installs,
            });
            match super::model_cluster_service::create(&ctx, &principal, body).await {
                Ok(created) => result["cluster_id"] = created["id"].clone(),
                Err(error) => result["cluster_error"] = error.message.clone().into(),
            }
        }
    }

    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let job = job_store::finish(&mut scope, job_id, Some(&result))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    ctx.bus.publish(LiveEvent::JobProgress {
        job_id,
        environment_id,
        completed: job.completed + job.failed,
        total: job.total,
        status: job.status.as_str().to_string(),
    });
    Ok(())
}

/// Start the install on one node and wait until it is installed, failed or out of time.
async fn install_one(
    ctx: &ServiceContext,
    principal: &Principal,
    key: &str,
    node: Node,
    force: bool,
    limit_seconds: u64,
) -> Outcome {
    let started = match super::model_service::install(ctx, principal, key, Some(&node.name), force)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            return Outcome { node, ok: false, install_id: None, message: error.message };
        }
    };
    let install_id = started["install_id"].as_str().and_then(|raw| Uuid::parse_str(raw).ok());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(limit_seconds);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_SECONDS)).await;
        let current = match TenantScope::begin(&ctx.pool, principal.environment_id).await {
            Ok(mut scope) => {
                let row = install_store::get(&mut scope, key, node.id).await;
                let _ = scope.commit().await;
                row
            }
            Err(error) => Err(error),
        };
        match current {
            Ok(Some(install)) => match install.state {
                InstallState::Installed => {
                    return Outcome {
                        node,
                        ok: true,
                        install_id: Some(install.id),
                        message: "installed".to_string(),
                    };
                }
                InstallState::Failed => {
                    return Outcome {
                        node,
                        ok: false,
                        install_id: Some(install.id),
                        message: install.error.unwrap_or_else(|| "the install failed".to_string()),
                    };
                }
                _ => {}
            },
            Ok(None) => {
                return Outcome {
                    node,
                    ok: false,
                    install_id,
                    message: "the install disappeared while waiting".to_string(),
                };
            }
            Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Outcome {
                node,
                ok: false,
                install_id,
                message: format!("the install did not finish within {limit_seconds} s"),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gpu_and_installed_nodes_are_skipped() {
        assert_eq!(skip_reason(Fit::NoGpu), Some("no_gpu"));
        assert_eq!(skip_reason(Fit::AlreadyInstalled), Some("installed"));
        assert_eq!(skip_reason(Fit::InsufficientVram), Some("too_small"));
        assert_eq!(skip_reason(Fit::Installable), None);
        assert_eq!(skip_reason(Fit::CpuInstallable), None);
    }

    #[test]
    fn the_wait_scales_with_the_weights_but_never_drops_below_ten_minutes() {
        assert_eq!(wait_limit_seconds(None), 600);
        assert_eq!(wait_limit_seconds(Some(1024)), 600);
        let forty_gib = 40 * 1024 * 1024 * 1024;
        assert_eq!(wait_limit_seconds(Some(forty_gib)), 2048);
    }

    #[test]
    fn a_fitting_verdict_reads_fits() {
        assert_eq!(fit_label(Fit::Installable), "fits");
        assert_eq!(fit_label(Fit::NoGpu), "no_gpu");
    }
}
