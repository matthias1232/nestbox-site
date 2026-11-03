//! Bulk sandbox actions as one job (package B03, contract C5).
//!
//! One request names many sandboxes and one action. The request is validated up front, then a
//! job runs the action per sandbox with the caller's own principal, so permissions and row-level
//! security apply to every target. A target that fails is recorded and the loop goes on.

use std::sync::Arc;

use uuid::Uuid;

use super::sandbox_service::{self, LifecycleAction};
use super::{egress_live_service, move_service, policy_service, ServiceContext};
use crate::config::Config;
use crate::domain::auth::Principal;
use crate::domain::job::{JobKind, TargetState};
use crate::domain::policy::EgressMode;
use crate::domain::telemetry::LiveEvent;
use crate::error::ApiError;
use crate::store::postgres::jobs as job_store;
use crate::store::TenantScope;

/// The most sandboxes one request may name.
const MAX_TARGETS: usize = 500;

/// Minutes an `allow_all` lasts when the request gives none.
const DEFAULT_ALLOW_MINUTES: i32 = 60;

/// An action after its parameters were checked; cheap to copy into each target's turn.
#[derive(Debug, Clone)]
enum Plan {
    Lifecycle(LifecycleAction),
    Recreate,
    AllowAll { minutes: i32 },
    EgressMode(EgressMode),
    AssignPolicy { policy_id: Uuid },
    Move { node: String },
    Delete,
}

/// Check the action and its parameters before anything is written.
fn plan(action: &str, params: &serde_json::Value) -> Result<Plan, ApiError> {
    match action {
        "start" | "stop" | "restart" => LifecycleAction::parse(action)
            .map(Plan::Lifecycle)
            .ok_or_else(|| ApiError::validation("Unknown action.")),
        "recreate" => Ok(Plan::Recreate),
        "allow_all" => {
            let minutes = match params.get("minutes") {
                None | Some(serde_json::Value::Null) => DEFAULT_ALLOW_MINUTES,
                Some(value) => value
                    .as_i64()
                    .filter(|m| (1..=1440).contains(m))
                    .ok_or_else(|| ApiError::validation("Allow all lasts 1 to 1440 minutes."))?
                    as i32,
            };
            Ok(Plan::AllowAll { minutes })
        }
        "egress_mode" => {
            let mode = params
                .get("mode")
                .and_then(|v| v.as_str())
                .and_then(EgressMode::parse)
                .ok_or_else(|| {
                    ApiError::validation(
                        "An egress mode is transparent, hybrid, forward_proxy, off_allow_all or direct.",
                    )
                })?;
            Ok(Plan::EgressMode(mode))
        }
        "assign_policy" => {
            let policy_id = params
                .get("policy_id")
                .and_then(|v| v.as_str())
                .and_then(|v| Uuid::parse_str(v).ok())
                .ok_or_else(|| ApiError::validation("Assigning a policy needs params.policy_id."))?;
            Ok(Plan::AssignPolicy { policy_id })
        }
        "move" => {
            let node = params
                .get("node")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| ApiError::validation("Moving needs params.node."))?;
            Ok(Plan::Move { node: node.to_string() })
        }
        "delete" => {
            if params.get("confirm").and_then(|v| v.as_str()) != Some("delete") {
                return Err(ApiError::validation(
                    "Deleting needs params.confirm set to \"delete\".",
                ));
            }
            Ok(Plan::Delete)
        }
        _ => Err(ApiError::validation(
            "An action is start, stop, restart, recreate, allow_all, egress_mode, assign_policy, move or delete.",
        )),
    }
}

/// Validate the ids and the request as a whole; returns the plan and the de-duplicated ids.
fn validate(
    ids: Vec<Uuid>,
    action: &str,
    params: &serde_json::Value,
) -> Result<(Plan, Vec<Uuid>), ApiError> {
    if ids.is_empty() || ids.len() > MAX_TARGETS {
        return Err(ApiError::validation("Name 1 to 500 sandboxes."));
    }
    let plan = plan(action, params)?;
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<Uuid> = ids.into_iter().filter(|id| seen.insert(*id)).collect();
    Ok((plan, unique))
}

/// Run the planned action on one sandbox and say what happened.
async fn run_one(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    sandbox_id: Uuid,
    plan: &Plan,
) -> Result<String, ApiError> {
    match plan {
        Plan::Lifecycle(action) => {
            sandbox_service::act(ctx, principal, sandbox_id, *action).await?;
            Ok("done".to_string())
        }
        Plan::Recreate => {
            sandbox_service::recreate(ctx, principal, sandbox_id).await?;
            Ok("recreated".to_string())
        }
        Plan::AllowAll { minutes } => {
            policy_service::open_window(
                ctx,
                principal,
                Some(sandbox_id),
                Some(*minutes),
                "bypass",
                "all",
                "bulk allow all",
            )
            .await?;
            Ok(format!("allow all for {minutes} minutes"))
        }
        Plan::EgressMode(mode) => {
            egress_live_service::apply(ctx, principal, sandbox_id, *mode, None).await?;
            Ok("egress mode changed".to_string())
        }
        Plan::AssignPolicy { policy_id } => {
            policy_service::assign_policy_to_sandbox(ctx, principal, sandbox_id, *policy_id)
                .await?;
            Ok("policy assigned".to_string())
        }
        Plan::Move { node } => {
            move_service::start(ctx, config, principal, sandbox_id, node).await?;
            Ok(format!("move to {node} started"))
        }
        Plan::Delete => {
            sandbox_service::delete(ctx, principal, sandbox_id).await?;
            Ok("deleted".to_string())
        }
    }
}

/// Start one job that applies `action` to every sandbox in `ids`; returns `{job_id, total}`.
///
/// The job runs in the background with the caller's principal. Each target gets one job log line
/// `{ "sandbox_id", "ok", "message" }` and one `JobProgress` event.
pub async fn sandboxes(
    ctx: &ServiceContext,
    config: Arc<Config>,
    principal: &Principal,
    ids: Vec<Uuid>,
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    let (plan, ids) = validate(ids, action, &params)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let payload = serde_json::json!({ "action": action, "ids": ids });
    let job = job_store::create(
        &mut scope,
        JobKind::SandboxBulkAction,
        &ids,
        &payload,
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let job_id = job.id;
    let total = ids.len();
    let ctx = ctx.clone();
    let principal = principal.clone();
    tokio::spawn(async move {
        run_job(&ctx, &config, &principal, job_id, ids, plan).await;
    });
    Ok(serde_json::json!({ "job_id": job_id, "total": total }))
}

/// The background loop: one target after the other, never aborting on a failed target.
async fn run_job(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    job_id: Uuid,
    ids: Vec<Uuid>,
    plan: Plan,
) {
    let environment_id = principal.environment_id;
    let total = ids.len() as i32;
    if let Ok(mut scope) = TenantScope::begin(&ctx.pool, environment_id).await {
        let _ = job_store::start(&mut scope, job_id).await;
        let _ = scope.commit().await;
    }
    let mut completed = 0_i32;
    for sandbox_id in ids {
        let outcome = run_one(ctx, config, principal, sandbox_id, &plan).await;
        let (ok, message) = match &outcome {
            Ok(message) => (true, message.clone()),
            Err(error) => (false, error.message.clone()),
        };
        let line = serde_json::json!({ "sandbox_id": sandbox_id, "ok": ok, "message": message });
        if let Ok(mut scope) = TenantScope::begin(&ctx.pool, environment_id).await {
            let state = if ok { TargetState::Succeeded } else { TargetState::Failed };
            let error = if ok { None } else { Some(message.as_str()) };
            let _ = job_store::record_target(&mut scope, job_id, sandbox_id, state, error).await;
            let _ = job_store::log(
                &mut scope,
                job_id,
                if ok { "info" } else { "error" },
                &line.to_string(),
            )
            .await;
            let _ = scope.commit().await;
        }
        completed += 1;
        ctx.bus.publish(LiveEvent::JobProgress {
            job_id,
            environment_id,
            completed,
            total,
            status: "running".to_string(),
        });
    }
    let mut status = "succeeded".to_string();
    if let Ok(mut scope) = TenantScope::begin(&ctx.pool, environment_id).await {
        if let Ok(job) = job_store::finish(&mut scope, job_id, None).await {
            status = job.status.as_str().to_string();
        }
        let _ = scope.commit().await;
    }
    ctx.bus.publish(LiveEvent::JobProgress {
        job_id,
        environment_id,
        completed,
        total,
        status,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_needs_its_confirmation() {
        let ids = vec![Uuid::nil()];
        assert!(validate(ids.clone(), "delete", &serde_json::json!({})).is_err());
        assert!(validate(ids, "delete", &serde_json::json!({ "confirm": "delete" })).is_ok());
    }

    #[test]
    fn the_id_list_is_bounded_and_deduplicated() {
        assert!(validate(vec![], "stop", &serde_json::json!({})).is_err());
        let many = vec![Uuid::nil(); 3];
        let (_, unique) = validate(many, "stop", &serde_json::json!({})).expect("valid");
        assert_eq!(unique.len(), 1);
    }

    #[test]
    fn allow_all_minutes_are_checked_and_default_to_an_hour() {
        assert!(plan("allow_all", &serde_json::json!({ "minutes": 0 })).is_err());
        assert!(plan("allow_all", &serde_json::json!({ "minutes": 1441 })).is_err());
        assert!(matches!(
            plan("allow_all", &serde_json::json!({ "minutes": null })),
            Ok(Plan::AllowAll { minutes: 60 })
        ));
    }

    #[test]
    fn move_egress_and_policy_need_their_parameters() {
        assert!(plan("move", &serde_json::json!({})).is_err());
        assert!(plan("egress_mode", &serde_json::json!({ "mode": "nope" })).is_err());
        assert!(plan("assign_policy", &serde_json::json!({})).is_err());
        assert!(plan("explode", &serde_json::json!({})).is_err());
    }
}
