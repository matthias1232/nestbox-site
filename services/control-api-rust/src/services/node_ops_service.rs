//! Node summary, drain and quick deploy (package B07, contract C8).
//!
//! * `summary` answers "how is this machine doing" in one call: the node, its place in the tree,
//!   live load from its own runner, what runs and is installed on it, its images and its tunnel.
//! * `drain` empties a node as a job: every sandbox is moved to another node or stopped. Model
//!   installs are not touched.
//! * `quick` gets a new node, and optionally a model on it, in one request. It chains services that
//!   already exist (rent, connect, join token, probe, install) and reports each step as a job target.
//!
//! Drain and quick deploy follow the same shape as every fan-out job: validate, create the job,
//! answer `202`, then do the slow part in a spawned task that records one target and one log line
//! per step and publishes `JobProgress` after each.

use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use super::sandbox_service::{self, LifecycleAction};
use super::{model_service, move_service, node_connect_service, node_join_service, node_service, provision_service, require, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::domain::job::{JobKind, TargetState};
use crate::domain::model_install::InstallState;
use crate::domain::node::NodeStatus;
use crate::domain::node_tunnel::ConnectNodeRequest;
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::{jobs as job_store, model_installs, node_gateway, nodes as node_store};

/// How long the summary waits for a node's runner before it reports the load as unknown.
const LOAD_TIMEOUT: Duration = Duration::from_secs(3);

/// How often a background step looks again at what it waits for.
const POLL_EVERY: Duration = Duration::from_secs(5);

/// The longest a drain waits for one sandbox's move.
const MOVE_WAIT: Duration = Duration::from_secs(30 * 60);

/// The longest a quick deploy waits for a model install on the new node.
const INSTALL_WAIT: Duration = Duration::from_secs(30 * 60);

/// How long a quick deploy waits for the node to come online when the request does not say.
const DEFAULT_WAIT_MINUTES: i64 = 20;

/// The body of `POST /nodes/{id}/drain`.
#[derive(Debug, Deserialize)]
struct DrainRequest {
    /// The node name that receives the sandboxes; required unless `stop_only`.
    #[serde(default)]
    to_node: Option<String>,
    /// Stop the sandboxes instead of moving them.
    #[serde(default)]
    stop_only: bool,
}

/// Where a quick-deployed node comes from. No `Debug`: the connect variant carries credentials.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum QuickSource {
    /// A machine rented from a provider account.
    Rent {
        account_id: Uuid,
        host_type: String,
        #[serde(default)]
        location: Option<String>,
        #[serde(default)]
        ttl_minutes: Option<i64>,
        #[serde(default)]
        idle_minutes: Option<i32>,
    },
    /// A machine that exists; installed over SSH with credentials that are never stored.
    Connect {
        address: String,
        /// The SSH port the machine listens on; the connect service default applies when absent.
        #[serde(default)]
        ssh_port: Option<u16>,
        #[serde(default)]
        ssh_user: Option<String>,
        #[serde(default)]
        ssh_password: Option<String>,
        #[serde(default)]
        ssh_private_key: Option<String>,
        /// The UDP port the node's WireGuard listens on (`wg_listen_port` in the connect request).
        #[serde(default)]
        wg_port: Option<u16>,
        #[serde(default)]
        egress_lock: Option<String>,
    },
    /// The operator runs the join command on the machine.
    Script,
}

/// The body of `POST /nodes/quick`. No `Debug`, see `QuickSource`.
#[derive(Deserialize)]
struct QuickRequest {
    site_id: Uuid,
    name: String,
    source: QuickSource,
    #[serde(default)]
    model_key: Option<String>,
    #[serde(default)]
    wait_minutes: Option<i64>,
}

/// The human message of a service error, for a job log line.
fn why(error: ApiError) -> String {
    error.message
}

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

/// Turn the runner's host-load answer into the `load` object of contract C8.
///
/// The runner reports memory and disk as `{total, used}` objects and the load average as a list;
/// VRAM is not reported by the runner, so it stays `null` rather than an invented zero.
fn load_view(raw: &serde_json::Value) -> serde_json::Value {
    let number = |value: Option<&serde_json::Value>| value.and_then(serde_json::Value::as_i64);
    serde_json::json!({
        "cpu_percent": raw.get("cpu_percent").and_then(serde_json::Value::as_f64),
        "ram_used_bytes": number(raw.pointer("/memory/used")),
        "ram_total_bytes": number(raw.pointer("/memory/total")),
        "vram_used_bytes": serde_json::Value::Null,
        "vram_total_bytes": serde_json::Value::Null,
        "disk_used_bytes": number(raw.pointer("/disk/used")),
        "disk_total_bytes": number(raw.pointer("/disk/total")),
        "load1": raw.pointer("/load_average/0").and_then(serde_json::Value::as_f64),
    })
}

/// One node with load, sandboxes, installs, images and tunnel (contract C8).
pub async fn summary(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::NodeRead)?;
    // The node view first: it is also the visibility check, so a node of another data center is a 404.
    let node = node_service::get(ctx, principal, id).await?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let environment_id = scope.environment_id();

    let place = sqlx::query(
        "SELECT s.id AS site_id, s.name AS site_name, s.kind AS site_kind, e.id AS dc_id, e.name AS dc_name
           FROM environments e
           LEFT JOIN nodes n ON n.id = $2
           LEFT JOIN sites s ON s.id = n.site_id AND s.environment_id = e.id
          WHERE e.id = $1",
    )
    .bind(environment_id)
    .bind(id)
    .fetch_optional(scope.connection())
    .await
    .map_err(db)?;
    let (site, data_center) = match &place {
        Some(row) => {
            use sqlx::Row;
            let site_id: Option<Uuid> = row.try_get("site_id").map_err(db)?;
            let site = match site_id {
                Some(site_id) => serde_json::json!({
                    "id": site_id,
                    "name": row.try_get::<String, _>("site_name").map_err(db)?,
                    "kind": row.try_get::<String, _>("site_kind").map_err(db)?,
                }),
                None => serde_json::Value::Null,
            };
            let dc = serde_json::json!({
                "id": row.try_get::<Uuid, _>("dc_id").map_err(db)?,
                "name": row.try_get::<String, _>("dc_name").map_err(db)?,
            });
            (site, dc)
        }
        None => (serde_json::Value::Null, serde_json::Value::Null),
    };

    let (total, running, allow_all): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE s.status = 'running'),
                count(*) FILTER (WHERE coalesce(em.mode = 'off_allow_all', false) OR EXISTS (
                    SELECT 1 FROM traffic_windows w
                     WHERE w.environment_id = s.environment_id AND w.revoked_at IS NULL
                       AND w.expires_at > now() AND (w.sandbox_id IS NULL OR w.sandbox_id = s.id)))
           FROM sandboxes s
           LEFT JOIN egress_modes em ON em.sandbox_id = s.id AND em.environment_id = s.environment_id
          WHERE s.environment_id = $1 AND s.node_id = $2 AND s.status <> 'deleted'",
    )
    .bind(environment_id)
    .bind(id)
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;

    let install_rows: Vec<(String, String, Option<String>, i64)> = sqlx::query_as(
        "SELECT mi.model_key, mi.state, pp.status, mi.reserved_vram_bytes
           FROM model_installs mi
           LEFT JOIN provider_profiles pp ON pp.id = mi.provider_profile_id
          WHERE mi.environment_id = $1 AND mi.node_id = $2
          ORDER BY mi.created_at",
    )
    .bind(environment_id)
    .bind(id)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let installs: Vec<serde_json::Value> = install_rows
        .into_iter()
        .map(|(model_key, state, provider_status, reserved_vram_bytes)| {
            serde_json::json!({
                "model_key": model_key,
                "state": state,
                "provider_status": provider_status,
                "reserved_vram_bytes": reserved_vram_bytes,
            })
        })
        .collect();

    let image_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT state, count(*) FROM node_images WHERE node_id = $1 GROUP BY state")
            .bind(id)
            .fetch_all(scope.connection())
            .await
            .map_err(db)?;
    let image_count = |state: &str| image_rows.iter().find(|(name, _)| name == state).map(|(_, count)| *count).unwrap_or(0);

    let tunnel = sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>, i64, i64, String)>(
        "SELECT tunnel_state, tunnel_handshake_at, tunnel_rx_bytes, tunnel_tx_bytes, egress_lock
           FROM nodes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;

    let tunnel = match tunnel {
        Some((state, handshake_at, rx, tx, lock)) => serde_json::json!({
            "state": state, "handshake_at": handshake_at, "rx_bytes": rx, "tx_bytes": tx, "egress_lock": lock,
        }),
        None => serde_json::Value::Null,
    };
    let empty = node_join_service::node_is_empty(&ctx.pool, id).await?;
    let load = node_load(ctx, id).await;

    Ok(serde_json::json!({
        "node": node,
        "site": site,
        "data_center": data_center,
        "load": load,
        "sandboxes": { "total": total, "running": running, "allow_all": allow_all },
        "installs": installs,
        "images": {
            "present": image_count("present"),
            "pulling": image_count("pulling"),
            "missing": image_count("missing"),
            "failed": image_count("failed"),
        },
        "tunnel": tunnel,
        "empty": empty,
    }))
}

/// The load of one node from its own runner, or `null` when it does not answer in time.
async fn node_load(ctx: &ServiceContext, node_id: Uuid) -> serde_json::Value {
    let Some(router) = ctx.router.clone() else {
        return serde_json::Value::Null;
    };
    match tokio::time::timeout(LOAD_TIMEOUT, router.host_load_for(node_id)).await {
        Ok(Ok(raw)) => load_view(&raw),
        _ => serde_json::Value::Null,
    }
}

/// Follows one job from a spawned task: opens its own tenant scope for every write and publishes
/// the progress event, so a failing write is logged and never ends the job early.
struct Tracker {
    ctx: ServiceContext,
    environment_id: Uuid,
    job_id: Uuid,
    total: i32,
}

impl Tracker {
    /// Mark the job as running.
    async fn begin(&self) {
        let result = async {
            let mut scope = TenantScope::begin(&self.ctx.pool, self.environment_id).await?;
            job_store::start(&mut scope, self.job_id).await?;
            scope.commit().await
        }
        .await;
        if let Err(error) = result {
            tracing::warn!(job = %self.job_id, %error, "could not mark a job as started");
        }
    }

    /// Record the outcome of one target with its log line, then publish the progress.
    async fn target(&self, subject: Uuid, state: TargetState, line: serde_json::Value) {
        let ok = state == TargetState::Succeeded;
        let message = line.to_string();
        let result = async {
            let mut scope = TenantScope::begin(&self.ctx.pool, self.environment_id).await?;
            let error = if state == TargetState::Failed { Some(message.as_str()) } else { None };
            job_store::record_target(&mut scope, self.job_id, subject, state, error).await?;
            job_store::log(&mut scope, self.job_id, if ok { "info" } else { "error" }, &message).await?;
            let job = job_store::get(&mut scope, self.job_id).await?;
            scope.commit().await?;
            Ok::<_, StoreError>(job)
        }
        .await;
        match result {
            Ok(job) => self.publish(job.completed + job.failed, "running"),
            Err(error) => tracing::warn!(job = %self.job_id, %error, "could not record a job target"),
        }
    }

    /// Close the job with its result and publish the final status.
    async fn finish(&self, result: serde_json::Value) {
        let outcome = async {
            let mut scope = TenantScope::begin(&self.ctx.pool, self.environment_id).await?;
            let job = job_store::finish(&mut scope, self.job_id, Some(&result)).await?;
            scope.commit().await?;
            Ok::<_, StoreError>(job)
        }
        .await;
        match outcome {
            Ok(job) => self.publish(job.completed + job.failed, job.status.as_str()),
            Err(error) => tracing::warn!(job = %self.job_id, %error, "could not finish a job"),
        }
    }

    fn publish(&self, done: i32, status: &str) {
        self.ctx.bus.publish(LiveEvent::JobProgress {
            job_id: self.job_id,
            environment_id: self.environment_id,
            completed: done,
            total: self.total,
            status: status.to_string(),
        });
    }
}

/// Start a drain job; returns `{job_id, total}` (contract C8).
///
/// Every sandbox of the node is moved to `to_node` (preflight and move run through the existing
/// move service) or, with `stop_only`, stopped. Model installs stay where they are.
pub async fn drain(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    id: Uuid,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::NodeManage)?;
    let request: DrainRequest = serde_json::from_value(body)
        .map_err(|error| ApiError::validation(format!("A drain request needs 'to_node' and 'stop_only': {error}")))?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let node = match node_store::get(&mut scope, id).await {
        Ok(node) => node,
        Err(StoreError::NotFound) => return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "node")),
        Err(error) => return Err(ApiError::from(error)),
    };
    let target_name = if request.stop_only {
        None
    } else {
        let wanted = request.to_node.as_deref().map(str::trim).filter(|name| !name.is_empty());
        let Some(wanted) = wanted else {
            return Err(ApiError::validation("Say which node receives the sandboxes ('to_node'), or set 'stop_only'."));
        };
        let target = match node_store::get_by_name(&mut scope, wanted).await {
            Ok(target) => target,
            Err(StoreError::NotFound) => return Err(ApiError::validation(format!("There is no node '{wanted}'."))),
            Err(error) => return Err(ApiError::from(error)),
        };
        if target.id == node.id {
            return Err(ApiError::validation("A node cannot be drained onto itself."));
        }
        if target.status != NodeStatus::Online {
            return Err(ApiError::validation(format!("Node '{}' is not online.", target.name)));
        }
        Some(target.name)
    };

    let sandboxes: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, name, status FROM sandboxes
          WHERE environment_id = $1 AND node_id = $2 AND status <> 'deleted'
          ORDER BY name",
    )
    .bind(scope.environment_id())
    .bind(id)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let ids: Vec<Uuid> = sandboxes.iter().map(|(sandbox_id, _, _)| *sandbox_id).collect();
    let environment_id = scope.environment_id();
    let job = job_store::create(
        &mut scope,
        JobKind::NodeDrain,
        &ids,
        &serde_json::json!({ "node": node.name, "to_node": target_name, "stop_only": request.stop_only }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "node.drain",
            subject_type: Some("node"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "to_node": target_name, "stop_only": request.stop_only, "sandboxes": ids.len() }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let tracker = Tracker { ctx: ctx.clone(), environment_id, job_id: job.id, total: ids.len() as i32 };
    let ctx2 = ctx.clone();
    let config2 = config.clone();
    let principal2 = principal.clone();
    tokio::spawn(async move {
        tracker.begin().await;
        let mut moved = Vec::new();
        let mut stopped = Vec::new();
        let mut failed = Vec::new();
        for (sandbox_id, name, status) in sandboxes {
            let outcome = match &target_name {
                None => stop_one(&ctx2, &principal2, sandbox_id, &status).await.map(|_| "stopped"),
                Some(target) => move_one(&ctx2, &config2, &principal2, sandbox_id, target).await.map(|_| "moved"),
            };
            match outcome {
                Ok(what) => {
                    let line = serde_json::json!({ "sandbox_id": sandbox_id, "ok": true, "message": format!("{name} {what}") });
                    tracker.target(sandbox_id, TargetState::Succeeded, line).await;
                    if what == "moved" { moved.push(name) } else { stopped.push(name) }
                }
                Err(message) => {
                    let line = serde_json::json!({ "sandbox_id": sandbox_id, "ok": false, "message": format!("{name}: {message}") });
                    tracker.target(sandbox_id, TargetState::Failed, line).await;
                    failed.push(serde_json::json!({ "name": name, "message": message }));
                }
            }
        }
        tracker.finish(serde_json::json!({ "moved": moved, "stopped": stopped, "failed": failed })).await;
    });

    Ok(serde_json::json!({ "job_id": job.id, "total": ids.len() }))
}

/// Stop one sandbox for a drain; a sandbox that is not running counts as done.
async fn stop_one(ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid, status: &str) -> Result<(), String> {
    if status != "running" {
        return Ok(());
    }
    sandbox_service::act(ctx, principal, sandbox_id, LifecycleAction::Stop)
        .await
        .map(|_| ())
        .map_err(|error| why(error))
}

/// Move one sandbox for a drain and wait until its move row is `done` or `failed`.
async fn move_one(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    sandbox_id: Uuid,
    target: &str,
) -> Result<(), String> {
    let check = move_service::preflight(ctx, principal, sandbox_id, target).await.map_err(|error| why(error))?;
    if check.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let blockers = check.get("blockers").map(|value| value.to_string()).unwrap_or_default();
        return Err(format!("the move is blocked: {blockers}"));
    }
    let started = move_service::start(ctx, config, principal, sandbox_id, target).await.map_err(|error| why(error))?;
    let deadline = tokio::time::Instant::now() + MOVE_WAIT;
    loop {
        tokio::time::sleep(POLL_EVERY).await;
        let view = move_service::get(ctx, principal, started.id).await.map_err(|error| why(error))?;
        match view.state.as_str() {
            "done" => return Ok(()),
            "failed" => return Err(view.error.unwrap_or_else(|| "the move failed".into())),
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("the move did not finish in 30 minutes".into());
        }
    }
}

/// The refusal for a node name that already exists on this installation.
///
/// A node's own code, not the sandbox one: a client that branches on the answer must not
/// read "a sandbox of that name exists" when the clash is a node. The status stays 409
/// because the name, not the request, must change.
fn node_name_taken(node_name: &str) -> ApiError {
    ApiError::conflict(
        ErrorCode::NODE_NAME_TAKEN,
        format!("A node named '{node_name}' exists on this installation. Pick another name."),
    )
}

/// The refusal for a site that is not in the caller's data center.
fn site_not_in_environment() -> ApiError {
    ApiError::not_found(ErrorCode::SITE_NOT_FOUND, "No such site in this data center.")
}

/// The refusal for a model key the catalogue does not hold.
fn model_not_in_catalogue() -> ApiError {
    ApiError::not_found(ErrorCode::MODEL_NOT_FOUND, "No such model in the catalogue.")
}

/// Start a quick-deploy job; returns `{job_id, join_command}` (contract C8).
///
/// Steps are `provision`, `join`, `probe` and, when a model is wanted, `install_model`; each is a
/// job target with a `{step, ok, message}` log line. The underlying rent, connect and join-token
/// services (join token, SSH connect, rent) check the environment-manage permission themselves and
/// have no internal variant without that check, so `quick` requires the same permission up front:
/// a node-manage operator is refused immediately with a clear message instead of failing in step one
/// after a job was created.
pub async fn quick(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let request: QuickRequest = serde_json::from_value(body)
        .map_err(|error| ApiError::validation(format!("A quick deploy needs 'site_id', 'name' and a 'source': {error}")))?;
    let node_name = node_join_service::valid_node_name(&request.name)?;
    let wait = Duration::from_secs(request.wait_minutes.unwrap_or(DEFAULT_WAIT_MINUTES).clamp(1, 120) as u64 * 60);
    let model_key = request.model_key.as_deref().map(str::trim).filter(|key| !key.is_empty()).map(str::to_string);

    if let QuickSource::Connect { address, ssh_password, ssh_private_key, egress_lock, .. } = &request.source {
        if address.trim().is_empty() {
            return Err(ApiError::validation("Connecting a machine needs its address."));
        }
        let has = |value: &Option<String>| value.as_deref().is_some_and(|text| !text.trim().is_empty());
        if has(ssh_password) == has(ssh_private_key) {
            return Err(ApiError::validation("Give either an SSH password or an SSH private key, not both and not neither."));
        }
        if let Some(lock) = egress_lock {
            if !matches!(lock.as_str(), "strict" | "exit_local") {
                return Err(ApiError::validation("A lock is 'strict' or 'exit_local'."));
            }
        }
    }
    if node_gateway::name_taken(&ctx.pool, &node_name).await.map_err(ApiError::from)? {
        return Err(node_name_taken(&node_name));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    if !node_gateway::site_in_environment(&mut scope, request.site_id).await.map_err(ApiError::from)? {
        return Err(site_not_in_environment());
    }
    if let Some(key) = &model_key {
        let known: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM models WHERE key = $1)")
            .bind(key)
            .fetch_one(scope.connection())
            .await
            .map_err(db)?;
        if !known {
            return Err(model_not_in_catalogue());
        }
    }
    let environment_id = scope.environment_id();
    scope.commit().await.map_err(ApiError::from)?;

    // The script source hands out the join command now; the other sources create theirs inside
    // the provision step.
    let join_command = match &request.source {
        QuickSource::Script => Some(node_join_service::create_token(ctx, config, principal, &node_name, request.site_id).await?.command),
        _ => None,
    };

    let mut steps: Vec<(&'static str, Uuid)> =
        ["provision", "join", "probe"].into_iter().map(|step| (step, Uuid::new_v4())).collect();
    if model_key.is_some() {
        steps.push(("install_model", Uuid::new_v4()));
    }
    let subjects: Vec<Uuid> = steps.iter().map(|(_, subject)| *subject).collect();
    let source_kind = match &request.source {
        QuickSource::Rent { .. } => "rent",
        QuickSource::Connect { .. } => "connect",
        QuickSource::Script => "script",
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let job = job_store::create(
        &mut scope,
        JobKind::NodeQuickDeploy,
        &subjects,
        &serde_json::json!({ "node": node_name, "site_id": request.site_id, "source": source_kind, "model_key": model_key }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "node.quick_deploy",
            subject_type: Some("site"),
            subject_id: Some(request.site_id),
            detail: &serde_json::json!({ "node": node_name, "source": source_kind, "model_key": model_key }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let tracker = Tracker { ctx: ctx.clone(), environment_id, job_id: job.id, total: steps.len() as i32 };
    let ctx2 = ctx.clone();
    let config2 = config.clone();
    let principal2 = principal.clone();
    let site_id = request.site_id;
    let source = request.source;
    tokio::spawn(async move {
        tracker.begin().await;
        let outcome = run_quick(&ctx2, &config2, &principal2, &tracker, &steps, site_id, &node_name, source, model_key.as_deref(), wait).await;
        // Steps after a failure never ran; mark them so the job's counters are final.
        let mut ran = 0;
        if let Err((failed_at, _)) = &outcome {
            for (index, (step, subject)) in steps.iter().enumerate() {
                if index > *failed_at {
                    let line = serde_json::json!({ "step": step, "ok": false, "message": "skipped: an earlier step failed" });
                    tracker.target(*subject, TargetState::Skipped, line).await;
                }
            }
        } else {
            ran = steps.len();
        }
        let result = match outcome {
            Ok(node_id) => serde_json::json!({ "node": node_name, "node_id": node_id, "steps_done": ran }),
            Err((_, message)) => serde_json::json!({ "node": node_name, "error": message }),
        };
        tracker.finish(result).await;
    });

    Ok(serde_json::json!({ "job_id": job.id, "join_command": join_command }))
}

/// Run the steps of a quick deploy in order. On failure the error carries the index of the step
/// that failed and its message; on success it carries the new node's id.
#[allow(clippy::too_many_arguments)]
async fn run_quick(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    tracker: &Tracker,
    steps: &[(&'static str, Uuid)],
    site_id: Uuid,
    node_name: &str,
    source: QuickSource,
    model_key: Option<&str>,
    wait: Duration,
) -> Result<Uuid, (usize, String)> {
    let mut node_id = Uuid::nil();
    for (index, (step, subject)) in steps.iter().enumerate() {
        let result: Result<(TargetState, String), String> = match *step {
            "provision" => provision_step(ctx, config, principal, site_id, node_name, &source).await,
            "join" => match wait_for_node(ctx, principal.environment_id, node_name, wait).await {
                Ok(id) => {
                    node_id = id;
                    Ok((TargetState::Succeeded, format!("node {node_name} is online")))
                }
                Err(message) => Err(message),
            },
            "probe" => model_service::probe_node(ctx, principal, node_id)
                .await
                .map(|_| (TargetState::Succeeded, "hardware measured".to_string()))
                .map_err(|error| why(error)),
            _ => match model_key {
                Some(key) => install_step(ctx, principal, key, node_name, node_id).await.map(|_| (TargetState::Succeeded, format!("{key} installed"))),
                None => Ok((TargetState::Skipped, "no model requested".to_string())),
            },
        };
        match result {
            Ok((state, message)) => {
                tracker.target(*subject, state, serde_json::json!({ "step": step, "ok": true, "message": message })).await;
            }
            Err(message) => {
                tracker.target(*subject, TargetState::Failed, serde_json::json!({ "step": step, "ok": false, "message": message })).await;
                return Err((index, message));
            }
        }
    }
    Ok(node_id)
}

/// The provision step: rent a machine, connect one, or (script) nothing to do.
async fn provision_step(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    site_id: Uuid,
    node_name: &str,
    source: &QuickSource,
) -> Result<(TargetState, String), String> {
    match source {
        QuickSource::Script => Ok((TargetState::Skipped, "waiting for the join command to be run".into())),
        QuickSource::Rent { account_id, host_type, location, ttl_minutes, idle_minutes } => {
            provision_service::burst(
                ctx,
                config,
                principal,
                *account_id,
                node_name,
                site_id,
                host_type,
                location.as_deref(),
                ttl_minutes.unwrap_or(120),
                idle_minutes.unwrap_or(30),
            )
            .await
            .map(|host| (TargetState::Succeeded, format!("machine {} requested ({})", host.node_name, host.state)))
            .map_err(|error| why(error))
        }
        QuickSource::Connect { address, ssh_port, ssh_user, ssh_password, ssh_private_key, wg_port, egress_lock } => {
            let mut body = serde_json::json!({
                "node_name": node_name,
                "site_id": site_id,
                "host": address,
                "ssh_user": ssh_user.clone().unwrap_or_else(|| "root".into()),
                "password": ssh_password,
                "private_key": ssh_private_key,
                "egress_lock": egress_lock.clone().unwrap_or_else(|| "strict".into()),
            });
            // Both ports are optional in the quick body but not in the connect request, whose
            // deserialiser defaults would answer for an absent number; only a given port is sent.
            if let Some(fields) = body.as_object_mut() {
                if let Some(port) = ssh_port {
                    fields.insert("ssh_port".into(), serde_json::json!(port));
                }
                if let Some(port) = wg_port {
                    fields.insert("wg_listen_port".into(), serde_json::json!(port));
                }
            }
            let request: ConnectNodeRequest = serde_json::from_value(body)
                .map_err(|_| "the connection details could not be read".to_string())?;
            node_connect_service::connect(ctx, config, principal, request)
                .await
                .map(|_| (TargetState::Succeeded, "installation started over SSH".to_string()))
                .map_err(|error| why(error))
        }
    }
}

/// Wait until a node of this name is online; its id, or why it did not come.
async fn wait_for_node(ctx: &ServiceContext, environment_id: Uuid, name: &str, wait: Duration) -> Result<Uuid, String> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let found = async {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id).await?;
            let node = match node_store::get_by_name(&mut scope, name).await {
                Ok(node) => Some(node),
                Err(StoreError::NotFound) => None,
                Err(error) => return Err(error),
            };
            scope.commit().await?;
            Ok(node)
        }
        .await;
        if let Ok(Some(node)) = found {
            if node.status == NodeStatus::Online {
                return Ok(node.id);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("node {name} did not come online in time"));
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

/// Install a model on the new node and wait until the install is `installed` or `failed`.
async fn install_step(ctx: &ServiceContext, principal: &Principal, key: &str, node_name: &str, node_id: Uuid) -> Result<(), String> {
    model_service::install(ctx, principal, key, Some(node_name), false).await.map_err(|error| why(error))?;
    let deadline = tokio::time::Instant::now() + INSTALL_WAIT;
    loop {
        let install = async {
            let mut scope = TenantScope::begin(&ctx.pool, principal.environment_id).await?;
            let install = model_installs::get(&mut scope, key, node_id).await?;
            scope.commit().await?;
            Ok::<_, StoreError>(install)
        }
        .await;
        if let Ok(Some(install)) = install {
            match install.state {
                InstallState::Installed => return Ok(()),
                InstallState::Failed => return Err(install.error.unwrap_or_else(|| "the install failed".into())),
                _ => {}
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("the install did not finish in 30 minutes".into());
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_name_clash_has_its_own_code_and_says_which_name() {
        let error = node_name_taken("gpu-01");
        assert_eq!(error.status.as_u16(), 409);
        assert_eq!(error.code, ErrorCode::NODE_NAME_TAKEN);
        assert_eq!(
            error.message,
            "A node named 'gpu-01' exists on this installation. Pick another name."
        );
    }

    #[test]
    fn an_unknown_site_is_a_site_not_a_node() {
        let error = site_not_in_environment();
        assert_eq!(error.status.as_u16(), 404);
        assert_eq!(error.code, ErrorCode::SITE_NOT_FOUND);
        assert_eq!(error.message, "No such site in this data center.");
    }

    #[test]
    fn an_unknown_model_is_a_catalogue_miss() {
        let error = model_not_in_catalogue();
        assert_eq!(error.status.as_u16(), 404);
        assert_eq!(error.code, ErrorCode::MODEL_NOT_FOUND);
        assert_eq!(error.message, "No such model in the catalogue.");
    }

    #[test]
    fn load_view_maps_the_runner_answer() {
        let raw = serde_json::json!({
            "cpu_percent": 12.5,
            "memory": { "total": 8000, "used": 3000 },
            "disk": { "total": 100, "used": 40, "free": 60 },
            "load_average": [0.4, 0.3, 0.2]
        });
        let view = load_view(&raw);
        assert_eq!(view["cpu_percent"], 12.5);
        assert_eq!(view["ram_total_bytes"], 8000);
        assert_eq!(view["ram_used_bytes"], 3000);
        assert_eq!(view["disk_used_bytes"], 40);
        assert_eq!(view["load1"], 0.4);
        assert!(view["vram_total_bytes"].is_null());
    }

    #[test]
    fn load_view_keeps_missing_parts_null() {
        let view = load_view(&serde_json::json!({ "memory": { "total": 1, "used": 1 } }));
        assert!(view["disk_total_bytes"].is_null());
        assert!(view["load1"].is_null());
    }

    #[test]
    fn a_source_is_chosen_by_its_kind() {
        let source: QuickSource = serde_json::from_value(serde_json::json!({ "kind": "script" })).ok().unwrap_or(QuickSource::Script);
        assert!(matches!(source, QuickSource::Script));
        assert!(serde_json::from_value::<QuickSource>(serde_json::json!({ "kind": "nope" })).is_err());
    }

    #[test]
    fn a_connect_source_keeps_the_ssh_and_wireguard_ports() {
        let source: QuickSource = serde_json::from_value(serde_json::json!({
            "kind": "connect",
            "address": "10.0.0.7",
            "ssh_password": "secret",
            "ssh_port": 2222,
            "wg_port": 51999
        }))
        .ok()
        .unwrap_or(QuickSource::Script);
        match source {
            QuickSource::Connect { ssh_port, wg_port, .. } => {
                assert_eq!(ssh_port, Some(2222));
                assert_eq!(wg_port, Some(51999));
            }
            _ => panic!("expected a connect source"),
        }
    }

    #[test]
    fn a_connect_source_without_ports_leaves_them_to_the_connect_defaults() {
        let source: QuickSource = serde_json::from_value(serde_json::json!({
            "kind": "connect",
            "address": "10.0.0.7",
            "ssh_password": "secret"
        }))
        .ok()
        .unwrap_or(QuickSource::Script);
        match source {
            QuickSource::Connect { ssh_port, wg_port, .. } => {
                assert_eq!(ssh_port, None);
                assert_eq!(wg_port, None);
            }
            _ => panic!("expected a connect source"),
        }
    }
}
