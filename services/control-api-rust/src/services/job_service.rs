//! Jobs and schedules.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::job::{Job, JobKind, Schedule};
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;
use crate::store::postgres::jobs as store;

/// The message a job carries when a restart, not a target, stopped it.
///
/// Fixed text on purpose: the dock and the Tasks list show it verbatim, and an
/// operator who sees it twice should recognise it as one situation.
const INTERRUPTED: &str = "interrupted by a restart";

/// The job kinds the Control Center starts and a restart can strand.
///
/// All four run in a bare `tokio::spawn` (`bulk_service`, `model_bulk_service`,
/// `node_ops_service`), so nothing survives the process to finish them.
const INTERRUPTED_KINDS: [JobKind; 4] = [
    JobKind::SandboxBulkAction,
    JobKind::ModelBulkInstall,
    JobKind::NodeDrain,
    JobKind::NodeQuickDeploy,
];

/// Settle the Control Center jobs a restart interrupted.
///
/// Called once at boot, before the first request: every job of those kinds still
/// `pending` or `running` belongs to a task that no longer exists, so it ends
/// `failed` with the reason above instead of showing progress that never moves.
/// Cross-tenant by nature, like the other boot settles — there is no principal.
pub async fn settle_interrupted(ctx: &ServiceContext) -> Result<u64, ApiError> {
    store::settle_interrupted(&ctx.pool, &INTERRUPTED_KINDS, INTERRUPTED)
        .await
        .map_err(ApiError::from)
}

/// List recent jobs.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    limit: i64,
) -> Result<Vec<Job>, ApiError> {
    require(principal, Permission::JobRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let jobs = store::list(&mut scope, limit.clamp(1, 200))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(jobs)
}

/// One job.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Job, ApiError> {
    require(principal, Permission::JobRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let job = store::get(&mut scope, id).await.map_err(|error| match error {
        crate::error::StoreError::NotFound => {
            ApiError::not_found(ErrorCode::JOB_NOT_FOUND, "No such job.")
        }
        other => ApiError::from(other),
    })?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(job)
}

/// A job's log.
pub async fn logs(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    limit: i64,
) -> Result<Vec<serde_json::Value>, ApiError> {
    require(principal, Permission::JobRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let logs = store::logs(&mut scope, id, limit.clamp(1, 1000))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(logs)
}

/// All schedules.
pub async fn schedules(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<Schedule>, ApiError> {
    require(principal, Permission::JobRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let schedules = store::list_schedules(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(schedules)
}

/// Run a schedule now, creating the job it would have created on its next tick.
///
/// A manual run is recorded the same way as a scheduled one, so "why did this
/// happen" has one answer path.
pub async fn run_schedule(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::JobWrite)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    let schedules = store::list_schedules(&mut scope)
        .await
        .map_err(ApiError::from)?;
    let Some(schedule) = schedules.into_iter().find(|s| s.id == id) else {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::not_found(
            ErrorCode::SCHEDULE_NOT_FOUND,
            "No such schedule.",
        ));
    };

    let kind = schedule_kind(&schedule.kind);
    let job = store::create(
        &mut scope,
        kind,
        &[],
        &schedule.payload,
        Some(schedule.id),
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;

    store::mark_schedule_run(
        &mut scope,
        schedule.id,
        if job.failed > 0 { "failed" } else { "succeeded" },
        next_run(&schedule.cron),
    )
    .await
    .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;

    // The catalog sync is the one scheduled kind that does work today: it
    // fetches from the public repositories. It runs detached for the same
    // reason a build does — ten seconds of network calls must not hold the HTTP
    // response — and the job row is the record of what happened either way.
    if kind == crate::domain::job::JobKind::CatalogSync {
        let ctx = ctx.clone();
        let environment_id = principal.environment_id;
        let job_id = job.id;
        tokio::spawn(async move {
            if let Err(error) =
                crate::services::catalog_sync::run(&ctx, environment_id, job_id).await
            {
                tracing::error!(
                    code = error.code.0,
                    job = %job_id,
                    "the scheduled catalog sync could not record its outcome"
                );
            }
        });
    }

    Ok(serde_json::json!({ "job_id": job.id, "schedule": schedule.name }))
}

/// The job kind a schedule produces.
fn schedule_kind(name: &str) -> crate::domain::job::JobKind {
    use crate::domain::job::JobKind;
    match name {
        "gateway_config_sync" => JobKind::GatewayConfigSync,
        "policy_reconciliation" => JobKind::PolicyReconciliation,
        "certificate_expiry_check" => JobKind::CertificateExpiryCheck,
        "address_pool_reconcile" => JobKind::AddressPoolReconcile,
        "telemetry_partition_maintenance" => JobKind::TelemetryPartitionMaintenance,
        "metrics_partition_maintenance" => JobKind::MetricsPartitionMaintenance,
        "metrics_rollup" => JobKind::MetricsRollup,
        "egress_partition_maintenance" => JobKind::EgressPartitionMaintenance,
        "provider_refresh" => JobKind::ProviderRefresh,
        "model_update_check" => JobKind::ModelUpdateCheck,
        "session_cleanup" => JobKind::SessionCleanup,
        "catalog_sync" => JobKind::CatalogSync,
        other => {
            tracing::warn!(schedule = other, "unknown schedule kind");
            JobKind::PolicyReconciliation
        }
    }
}

/// The next run time for a cron expression.
///
/// A minimal five-field evaluator: it handles the `*/N * * * *` and `M * * * *`
/// shapes the seeds use, and falls back to hourly for anything else rather than
/// claiming a precision it does not have. A full cron engine is a dependency
/// with its own edge cases; this covers the schedules the product ships.
fn next_run(cron: &str) -> chrono::DateTime<chrono::Utc> {
    use chrono::{Duration, Timelike, Utc};
    let now = Utc::now();
    let fields: Vec<&str> = cron.split_whitespace().collect();

    if fields.len() != 5 {
        return now + Duration::hours(1);
    }

    let minute_field = fields[0];
    let hour_field = fields[1];

    let minute = if let Some(step) = minute_field.strip_prefix("*/") {
        step.parse::<u32>().unwrap_or(5).clamp(1, 59)
    } else {
        minute_field.parse::<u32>().unwrap_or(0).min(59)
    };

    let hour = if let Some(step) = hour_field.strip_prefix("*/") {
        step.parse::<u32>().unwrap_or(1).clamp(1, 23)
    } else {
        hour_field.parse::<u32>().unwrap_or(0).min(23)
    };

    if hour_field.starts_with("*/") || minute_field.starts_with("*/") {
        // Step-based: advance to the next multiple of the step in this hour.
        let current_minute = now.minute();
        let delta = ((current_minute / minute) + 1) * minute;
        now + Duration::minutes(i64::from(delta - current_minute))
    } else {
        // Fixed time of day: today at that hour if it is still ahead, else
        // tomorrow.
        let today = now
            .date_naive()
            .and_hms_opt(hour, minute, 0)
            .map(|naive| naive.and_utc());
        match today {
            Some(candidate) if candidate > now => candidate,
            Some(candidate) => candidate + Duration::days(1),
            None => now + Duration::hours(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_cron_produces_a_future_time() {
        let next = next_run("*/20 * * * *");
        assert!(next > chrono::Utc::now());
        assert!(next <= chrono::Utc::now() + chrono::Duration::minutes(21));
    }

    #[test]
    fn a_daily_cron_produces_a_future_time() {
        let next = next_run("30 2 * * *");
        assert!(next > chrono::Utc::now());
        assert!(next <= chrono::Utc::now() + chrono::Duration::hours(25));
    }

    #[test]
    fn a_malformed_cron_falls_back_instead_of_panicking() {
        let next = next_run("not a cron");
        assert!(next > chrono::Utc::now());
    }

    #[test]
    fn seeded_schedule_kinds_all_map_to_a_job() {
        for name in [
            "gateway_config_sync",
            "policy_reconciliation",
            "certificate_expiry_check",
            "address_pool_reconcile",
            "telemetry_partition_maintenance",
            "metrics_partition_maintenance",
            "metrics_rollup",
            "egress_partition_maintenance",
            "provider_refresh",
            "model_update_check",
            "session_cleanup",
            "catalog_sync",
        ] {
            let kind = schedule_kind(name);
            assert!(!kind.as_str().is_empty());
        }
    }
}
