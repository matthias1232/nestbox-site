//! Sandbox resource history: ingest, query, rollup, retention.
//!
//! The rules this module owns, so they are in one place rather than in a query:
//!
//! * a batch from a node is accepted only for sandboxes in that node's
//!   environment, and a sample older than the raw retention is dropped rather
//!   than written into a partition that is about to disappear;
//! * `range=1h` reads raw samples, `24h` and `7d` read the one-minute rollup —
//!   the console gets the number of points it asked for, not a million;
//! * the rollup job is idempotent: re-running it for a bucket overwrites the
//!   same row with the same aggregate;
//! * the partition job creates the days ahead and drops the days behind, so the
//!   raw table has a ceiling instead of a growth rate.
//!
//! Phase 1 has one runtime adapter, so "the node's environment" and "the
//! caller's environment" are the same filter. A second adapter would add its
//! `node_id` to [`store::resolve_sandbox_names`] rather than a second path here.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::job::Job;
use crate::domain::metrics::{
    MetricBatch, MetricRange, MetricSample, MetricSeries, MetricSource, ProcessTable,
};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::runtime::RuntimeError;
use crate::store::TenantScope;
use crate::store::postgres::metrics as store;
use crate::store::postgres::sandboxes as sandbox_store;

/// How long raw samples are kept. The maintenance job drops older partitions, so
/// a sample older than this would land in a partition that is about to vanish.
const RAW_RETENTION_HOURS: i64 = 24;

/// How far a node's clock may run ahead before its samples are refused.
///
/// A small skew is normal and harmless; a large one means a broken clock, and
/// accepting it would put points in the future that the console draws as a gap
/// on the right-hand edge of every graph.
const CLOCK_SKEW_MINUTES: i64 = 5;

/// How far back a rollup run reaches by default.
///
/// The job runs every five minutes and this covers six missed ticks, so one
/// skipped run is repaired by the next. A longer outage is a `window_minutes`
/// in the job payload, not a second code path.
const DEFAULT_ROLLUP_WINDOW_MINUTES: i64 = 30;

/// How long rollup buckets are kept, per ADR 0025.
const DEFAULT_ROLLUP_RETAIN_DAYS: i32 = 7;

/// How many days of raw partitions to create ahead of today.
const DEFAULT_PARTITION_DAYS_AHEAD: i32 = 2;

/// How many days of raw samples to keep.
const DEFAULT_RAW_RETAIN_DAYS: i32 = 1;

/// What one ingest batch did, so a caller sees what was lost and why.
#[derive(Debug, Clone, Default, Serialize)]
pub struct IngestOutcome {
    /// Rows actually written. A duplicate point is not an error and not counted.
    pub stored: u64,
    /// Samples older than the raw retention or ahead of the clock skew.
    pub dropped_out_of_window: usize,
    /// Samples whose numbers cannot be true, such as a negative byte count.
    pub dropped_implausible: usize,
    /// Samples naming a sandbox this environment does not have.
    pub dropped_unknown_sandbox: usize,
}

/// Accept one batch of raw samples from a node.
///
/// The caller is the node's token, which is why write permission is required:
/// a forged batch makes a graph lie, and the graph is what an operator uses to
/// decide whether to intervene.
pub async fn ingest(
    ctx: &ServiceContext,
    principal: &Principal,
    batch: MetricBatch,
) -> Result<IngestOutcome, ApiError> {
    require(principal, Permission::SandboxWrite)?;

    if batch.is_oversized() {
        return Err(ApiError::unprocessable(
            ErrorCode::PAYLOAD_TOO_LARGE,
            format!(
                "A metrics batch carries at most {} samples.",
                MetricBatch::MAX_SAMPLES
            ),
        ));
    }

    let now = Utc::now();
    let oldest = now - Duration::hours(RAW_RETENTION_HOURS);
    let newest = now + Duration::minutes(CLOCK_SKEW_MINUTES);

    let mut outcome = IngestOutcome::default();
    let mut kept = Vec::with_capacity(batch.samples.len());
    for point in batch.samples {
        if !point.is_plausible() {
            outcome.dropped_implausible += 1;
            continue;
        }
        if point.ts < oldest || point.ts > newest {
            outcome.dropped_out_of_window += 1;
            continue;
        }
        kept.push(point);
    }

    if kept.is_empty() {
        return Ok(outcome);
    }

    let mut names: Vec<String> = kept.iter().map(|point| point.sandbox.clone()).collect();
    names.sort();
    names.dedup();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let resolved: HashMap<String, Uuid> = store::resolve_sandbox_names(&mut scope, &names)
        .await
        .map_err(ApiError::from)?;

    let mut samples: Vec<MetricSample> = Vec::with_capacity(kept.len());
    for point in kept {
        match resolved.get(&point.sandbox) {
            Some(sandbox_id) => samples.push(point.into_sample(*sandbox_id)),
            None => outcome.dropped_unknown_sandbox += 1,
        }
    }

    outcome.stored = store::insert_samples(&mut scope, &samples)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(outcome)
}

/// The window's points for one sandbox.
///
/// The table is picked by the window, not by the caller: an hour is raw samples
/// because a spike must survive, and a day or a week is the rollup because 864
/// 000 rows is not an answer. A sandbox that does not exist in this environment
/// is a 404 rather than an empty series, so the console can tell "gone" from
/// "idle".
pub async fn series(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    range: MetricRange,
    step: Option<u32>,
) -> Result<MetricSeries, ApiError> {
    require(principal, Permission::SandboxRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => {
                ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox.")
            }
            other => ApiError::from(other),
        })?;

    let step_seconds = range.effective_step(step);
    let until = Utc::now();
    let since = until - range.duration();
    let limit = range.max_points() as i64;

    let points = if range.uses_raw() {
        store::raw_series(&mut scope, sandbox_id, since, until, step_seconds, limit)
            .await
            .map_err(ApiError::from)?
    } else {
        store::rollup_series(&mut scope, sandbox_id, since, until, step_seconds, limit)
            .await
            .map_err(ApiError::from)?
    };

    scope.commit().await.map_err(ApiError::from)?;

    Ok(MetricSeries {
        sandbox_id,
        range,
        step_seconds,
        source: if range.uses_raw() {
            MetricSource::Raw
        } else {
            MetricSource::Rollup
        },
        points,
    })
}

/// The sandbox's process table, read live through the runner.
///
/// Live and not stored: a process list is a now answer, and a stored one would
/// be a history nobody asked for. There is no pagination either — the runner caps
/// the table and says when it did.
pub async fn processes(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<ProcessTable, ApiError> {
    require(principal, Permission::SandboxRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => {
                ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox.")
            }
            other => ApiError::from(other),
        })?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.runtime
        .processes(&sandbox.name)
        .await
        .map_err(runner_error)
}

/// The sandbox a desktop session may attach to, checked before the upgrade.
///
/// Watching a desktop is as revealing as a terminal but not as powerful as one:
/// a viewer may drive the mouse and the keyboard, so driving needs
/// `SandboxExec`. Only watching needs `SandboxRead`, and the caller cannot
/// choose that on their own — `view_only` is requested, and the returned flag
/// says whether the session really is view-only. A principal without
/// `SandboxExec` is always view-only, even when the console asked for control.
/// Returning the name rather than a ticket is enough: nothing here is counted
/// or limited yet.
pub async fn desktop_target(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    view_only: bool,
) -> Result<(String, bool), ApiError> {
    if view_only {
        require(principal, Permission::SandboxRead)?;
    } else {
        require(principal, Permission::SandboxExec)?;
    }
    let effective_view_only = view_only || !principal.can(Permission::SandboxExec);
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id).await.map_err(|error| match error {
        StoreError::NotFound => ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox."),
        other => ApiError::from(other),
    })?;
    scope.commit().await.map_err(ApiError::from)?;
    if sandbox.status != crate::domain::sandbox::SandboxStatus::Running {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!("A desktop needs a running sandbox; this one is '{}'.", sandbox.status.as_str()),
        ));
    }
    Ok((sandbox.name, effective_view_only))
}

/// A live view of one sandbox, read through the runner.
///
/// The set of kinds is closed here: the kind becomes part of a runner URL, so
/// only a name this function knows may pass.
pub async fn live_view(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    kind: &str,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxRead)?;
    if !["connections", "commands"].contains(&kind) {
        return Err(ApiError::validation("Unknown live view."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id).await.map_err(|error| match error {
        StoreError::NotFound => ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox."),
        other => ApiError::from(other),
    })?;
    scope.commit().await.map_err(ApiError::from)?;
    ctx.runtime.live_view(&sandbox.name, kind).await.map_err(runner_error)
}

pub async fn process_action(
    ctx: &ServiceContext, principal: &Principal, sandbox_id: Uuid, pid: i64, action: String, nice: Option<i8>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    if pid < 2 { return Err(ApiError::validation("PID 1 is the sandbox entrypoint; use the sandbox controls instead.")); }
    let allowed = ["terminate", "kill", "pause", "resume", "set_nice"];
    if !allowed.contains(&action.as_str()) { return Err(ApiError::validation("Unknown process action.")); }
    if action == "set_nice" {
        let value = nice.ok_or_else(|| ApiError::validation("set_nice requires a nice value."))?;
        if !(-20..=19).contains(&value) { return Err(ApiError::validation("Nice must be between -20 and 19.")); }
        if value < 0 && principal.role != crate::domain::auth::Role::Admin { return Err(ApiError::forbidden("Only an admin may raise process priority.")); }
    } else if nice.is_some() { return Err(ApiError::validation("nice is only valid for set_nice.")); }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id).await.map_err(|error| match error { StoreError::NotFound => ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox."), other => ApiError::from(other) })?;
    scope.commit().await.map_err(ApiError::from)?;
    ctx.runtime.process_action(&sandbox.name, pid, &action, nice).await.map_err(runner_error)?;
    Ok(serde_json::json!({"pid": pid, "action": action, "message": format!("Process action applied to PID {pid}.")}))
}

/// Aggregate completed minutes into the rollup and prune what outlived it.
///
/// Called by the `metrics_rollup` job, so there is no principal: the environment
/// is the job's, which is what makes the write tenant-scoped without a user.
/// Idempotent by construction — the rollup is an upsert keyed on the minute, so
/// running it twice for the same window writes the same rows.
pub async fn rollup(
    ctx: &ServiceContext,
    environment_id: Uuid,
    window_minutes: i64,
    retain_days: i32,
) -> Result<serde_json::Value, ApiError> {
    let window = window_minutes.clamp(5, 1_440);
    let retain = retain_days.clamp(1, 30);
    let since = Utc::now() - Duration::minutes(window);

    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let written = store::rollup_minutes(&mut scope, since)
        .await
        .map_err(ApiError::from)?;
    let pruned = store::prune_rollup(&mut scope, retain)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "window_minutes": window,
        "retain_days": retain,
        "rows_written": written,
        "rows_pruned": pruned,
    }))
}

/// Create the raw partitions ahead of today and drop the days behind.
///
/// Partition DDL is not tenant data: the partition is one table for every
/// environment, so this runs on the pool rather than inside a tenant scope, the
/// same way telemetry maintenance does.
pub async fn maintain_partitions(
    ctx: &ServiceContext,
    days_ahead: i32,
    retain_days: i32,
) -> Result<serde_json::Value, ApiError> {
    let ahead = days_ahead.clamp(0, 31);
    let retain = retain_days.clamp(1, 30);

    let created = store::ensure_partitions(&ctx.pool, ahead)
        .await
        .map_err(ApiError::from)?;
    let dropped = store::drop_expired_partitions(&ctx.pool, retain)
        .await
        .map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "days_ahead": ahead,
        "retain_days": retain,
        "created": created,
        "dropped": dropped,
    }))
}

/// Run one of the metrics jobs.
///
/// This is the handler half of a `metrics_rollup` or
/// `metrics_partition_maintenance` job: it reads the payload, applies the
/// defaults from the seeded schedules, and returns what the job record stores as
/// its result.
///
/// `job_service` does not call it yet — there is no job dispatcher in the
/// control plane, so a schedule creates a `pending` row and stops there (the
/// same state telemetry maintenance is in). The function is public and total on
/// the job payload so the dispatcher, when it lands, has nothing to add.
pub async fn run_job(ctx: &ServiceContext, job: &Job) -> Result<serde_json::Value, ApiError> {
    match job.kind.as_str() {
        "metrics_rollup" => {
            rollup(
                ctx,
                job.environment_id,
                payload_i64(job, "window_minutes", DEFAULT_ROLLUP_WINDOW_MINUTES),
                payload_i64(job, "retain_days", i64::from(DEFAULT_ROLLUP_RETAIN_DAYS)) as i32,
            )
            .await
        }
        "metrics_partition_maintenance" => {
            maintain_partitions(
                ctx,
                payload_i64(job, "days_ahead", i64::from(DEFAULT_PARTITION_DAYS_AHEAD)) as i32,
                payload_i64(job, "retain_days", i64::from(DEFAULT_RAW_RETAIN_DAYS)) as i32,
            )
            .await
        }
        other => Err(ApiError::internal(format!(
            "metrics_service does not handle job kind '{other}'."
        ))),
    }
}

/// Whether this module runs a job kind.
///
/// The dispatcher asks this before it calls [`run_job`], so an unhandled kind is
/// a decision rather than an exception, and the set stays closed.
pub fn handles(kind: &str) -> bool {
    matches!(kind, "metrics_rollup" | "metrics_partition_maintenance")
}

/// A number from a job payload, or the schedule's default.
///
/// A payload is operator-editable JSON, so a wrong type is a fallback rather
/// than a 500: the job still runs, with the value the seeded schedule uses.
fn payload_i64(job: &Job, key: &str, fallback: i64) -> i64 {
    job.payload
        .get(key)
        .and_then(|value| value.as_i64())
        .unwrap_or(fallback)
}

/// Map a runner failure to the answer a client should see.
///
/// Shared with `inventory_service`, because both ask the runner a live question
/// and both must distinguish "the sandbox has no container" from "the node did
/// not answer" — the first is a 404, the second is a 503 the console retries.
pub(crate) fn runner_error(error: RuntimeError) -> ApiError {
    match error {
        RuntimeError::NotFound(_) => ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "The sandbox has no container on its node.",
        ),
        RuntimeError::Rejected(detail) => ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            "The sandbox cannot answer that right now.",
        )
        .with_internal(detail),
        RuntimeError::Unavailable(detail) => ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "The node did not answer. Try again in a moment.",
        )
        .with_internal(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::job::JobStatus;

    fn job(payload: serde_json::Value) -> Job {
        Job {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            kind: "metrics_rollup".into(),
            status: JobStatus::Pending,
            total: 0,
            completed: 0,
            failed: 0,
            payload,
            result: None,
            error: None,
            schedule_id: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn a_job_payload_overrides_the_default_and_a_wrong_type_does_not() {
        let with_value = job(serde_json::json!({ "retain_days": 3 }));
        assert_eq!(payload_i64(&with_value, "retain_days", 7), 3);

        let wrong_type = job(serde_json::json!({ "retain_days": "seven" }));
        assert_eq!(payload_i64(&wrong_type, "retain_days", 7), 7);

        let absent = job(serde_json::json!({}));
        assert_eq!(payload_i64(&absent, "retain_days", 7), 7);
    }

    #[test]
    fn only_the_two_metrics_jobs_are_this_modules_work() {
        assert!(handles("metrics_rollup"));
        assert!(handles("metrics_partition_maintenance"));
        // Another package's maintenance job must not silently run the wrong code.
        assert!(!handles("telemetry_partition_maintenance"));
        assert!(!handles(""));
    }
}
