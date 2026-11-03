//! Jobs, job targets, job logs, and schedules.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::job::{Job, JobKind, JobStatus, Schedule, TargetState};
use crate::error::StoreError;

const JOB_COLUMNS: &str = "id, environment_id, kind, status, total, completed, failed, \
     payload, result, error, schedule_id, created_at, started_at, finished_at, updated_at";

fn row_to_job(row: &sqlx::postgres::PgRow) -> Result<Job, StoreError> {
    let status_raw: String = row.try_get("status")?;
    Ok(Job {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        kind: row.try_get("kind")?,
        status: JobStatus::parse(&status_raw).unwrap_or(JobStatus::Pending),
        total: row.try_get("total")?,
        completed: row.try_get("completed")?,
        failed: row.try_get("failed")?,
        payload: row.try_get("payload")?,
        result: row.try_get("result")?,
        error: row.try_get("error")?,
        schedule_id: row.try_get("schedule_id")?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Create a job with its target set in one transaction.
pub async fn create(
    scope: &mut TenantScope<'_>,
    kind: JobKind,
    targets: &[Uuid],
    payload: &serde_json::Value,
    schedule_id: Option<Uuid>,
    created_by: Option<Uuid>,
) -> Result<Job, StoreError> {
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO jobs
                (environment_id, kind, status, total, payload, schedule_id, created_by)
            VALUES ($1, $2, 'pending', $3, $4, $5, $6)
            RETURNING {JOB_COLUMNS}
         )
         SELECT {JOB_COLUMNS} FROM inserted"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(kind.as_str())
        .bind(targets.len() as i32)
        .bind(payload)
        .bind(schedule_id)
        .bind(created_by)
        .fetch_one(scope.connection())
        .await?;
    let job = row_to_job(&row)?;

    for target in targets {
        sqlx::query(
            "INSERT INTO job_targets (job_id, subject_id) VALUES ($1, $2)
             ON CONFLICT (job_id, subject_id) DO NOTHING",
        )
        .bind(job.id)
        .bind(target)
        .execute(scope.connection())
        .await?;
    }

    Ok(job)
}

/// List jobs, newest first.
pub async fn list(scope: &mut TenantScope<'_>, limit: i64) -> Result<Vec<Job>, StoreError> {
    let sql = format!(
        "SELECT {JOB_COLUMNS} FROM jobs
          WHERE environment_id = $1
          ORDER BY created_at DESC, id DESC
          LIMIT $2"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(limit)
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_job(&row)?);
    }
    Ok(out)
}

/// One job.
pub async fn get(scope: &mut TenantScope<'_>, id: Uuid) -> Result<Job, StoreError> {
    let sql = format!(
        "SELECT {JOB_COLUMNS} FROM jobs WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_job(&row)
}

/// Mark a job as started.
pub async fn start(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE jobs SET status = 'running', started_at = COALESCE(started_at, now()),
                         updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Record a target outcome and recompute the job's counters.
///
/// The counter update is derived from the targets rather than incremented, so a
/// retried target cannot double-count.
pub async fn record_target(
    scope: &mut TenantScope<'_>,
    job_id: Uuid,
    subject_id: Uuid,
    state: TargetState,
    error: Option<&str>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE job_targets SET state = $3, error = $4, updated_at = now()
          WHERE job_id = $1 AND subject_id = $2",
    )
    .bind(job_id)
    .bind(subject_id)
    .bind(state.as_str())
    .bind(error)
    .execute(scope.connection())
    .await?;

    sqlx::query(
        "UPDATE jobs j
            SET completed = sub.ok, failed = sub.bad, updated_at = now()
           FROM (
              SELECT COUNT(*) FILTER (WHERE state = 'succeeded') AS ok,
                     COUNT(*) FILTER (WHERE state = 'failed') AS bad
                FROM job_targets WHERE job_id = $1
           ) sub
          WHERE j.environment_id = $2 AND j.id = $1",
    )
    .bind(job_id)
    .bind(scope.environment_id())
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Finish a job, choosing the terminal status from its counters.
pub async fn finish(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    result: Option<&serde_json::Value>,
) -> Result<Job, StoreError> {
    let sql = format!(
        "WITH finished AS (
            UPDATE jobs
               SET status = CASE
                       WHEN failed > 0 AND completed > 0 THEN 'partial'
                       WHEN failed > 0 THEN 'failed'
                       ELSE 'succeeded'
                   END,
                   result = $3,
                   finished_at = now(),
                   updated_at = now()
             WHERE environment_id = $1 AND id = $2
         RETURNING {JOB_COLUMNS}
         )
         SELECT {JOB_COLUMNS} FROM finished"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(result)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;
    row_to_job(&row)
}

/// Append a line to a job's log.
pub async fn log(
    scope: &mut TenantScope<'_>,
    job_id: Uuid,
    level: &str,
    message: &str,
) -> Result<(), StoreError> {
    // The message is user-visible and may quote a resource name, so it is
    // redacted like every other boundary.
    let safe = crate::security::redaction::redact(message);
    sqlx::query(
        "INSERT INTO job_logs (job_id, level, message)
         SELECT $1, $2, $3
          WHERE EXISTS (SELECT 1 FROM jobs WHERE id = $1 AND environment_id = $4)",
    )
    .bind(job_id)
    .bind(level)
    .bind(safe)
    .bind(scope.environment_id())
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Read a job's log.
pub async fn logs(
    scope: &mut TenantScope<'_>,
    job_id: Uuid,
    limit: i64,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let rows = sqlx::query(
        "SELECT l.ts, l.level, l.message
           FROM job_logs l
           JOIN jobs j ON j.id = l.job_id
          WHERE j.environment_id = $1 AND l.job_id = $2
          ORDER BY l.ts DESC
          LIMIT $3",
    )
    .bind(scope.environment_id())
    .bind(job_id)
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(serde_json::json!({
            "ts": row.try_get::<chrono::DateTime<chrono::Utc>, _>("ts")?,
            "level": row.try_get::<String, _>("level")?,
            "message": row.try_get::<String, _>("message")?,
        }));
    }
    Ok(out)
}

/// All schedules.
pub async fn list_schedules(scope: &mut TenantScope<'_>) -> Result<Vec<Schedule>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, environment_id, name, kind, cron, payload, enabled,
                last_run_at, next_run_at, last_status, created_at, updated_at
           FROM schedules WHERE environment_id = $1 ORDER BY name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(Schedule {
            id: row.try_get("id")?,
            environment_id: row.try_get("environment_id")?,
            name: row.try_get("name")?,
            kind: row.try_get("kind")?,
            cron: row.try_get("cron")?,
            payload: row.try_get("payload")?,
            enabled: row.try_get("enabled")?,
            last_run_at: row.try_get("last_run_at")?,
            next_run_at: row.try_get("next_run_at")?,
            last_status: row.try_get("last_status")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        });
    }
    Ok(out)
}

/// Settle the jobs of the given kinds that a restart interrupted.
///
/// A job that runs in a detached task — a bulk action, a bulk model install, a
/// drain, a quick deploy — has no owner that notices the process going away. A
/// restart therefore leaves its row `pending` or `running` forever, and the dock
/// names it as the newest running task for good.
///
/// The pool, not a [`TenantScope`], because the boot has no principal and the
/// settle has to reach every data center. It stays narrow: only the kinds the
/// caller names, only rows still in flight, and only this table. `pending` is
/// this schema's "queued" (`jobs_status_valid`, migration 0006), the state a job
/// is created in before its task calls `start`.
pub async fn settle_interrupted(
    pool: &sqlx::PgPool,
    kinds: &[JobKind],
    error: &str,
) -> Result<u64, StoreError> {
    if kinds.is_empty() {
        return Ok(0);
    }
    let kinds: Vec<String> = kinds.iter().map(|kind| kind.as_str().to_string()).collect();
    let done = sqlx::query(
        "UPDATE jobs
            SET status = 'failed', error = $2, finished_at = now(), updated_at = now()
          WHERE kind = ANY($1) AND status IN ('pending', 'running')",
    )
    .bind(&kinds)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Record a schedule run.
pub async fn mark_schedule_run(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    status: &str,
    next_run_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE schedules
            SET last_run_at = now(), last_status = $3, next_run_at = $4, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(status)
    .bind(next_run_at)
    .execute(scope.connection())
    .await?;
    Ok(())
}
