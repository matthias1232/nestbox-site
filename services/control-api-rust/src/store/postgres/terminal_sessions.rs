//! Terminal sessions in `sandbox_sessions`.
//!
//! One row per open WebSocket: who, in which sandbox, when it opened, when it
//! closed, why, and how many bytes crossed. There is no column for content and
//! there will not be one (ADR 0023) — the table is the answer to "who had a
//! shell there", not a transcript.
//!
//! The same rows are the concurrency limit. Counting open sessions for a
//! sandbox and for a user before inserting one means the limit survives a
//! restart and holds across API replicas, which a counter in memory does not.
//! The cost is that a crash leaves a row open; `count_open_*` therefore ignores
//! rows older than a session can possibly live, so an abandoned row cannot
//! lock an operator out forever.

use std::time::Duration;

use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;

/// The kind this module writes. `ssh` belongs to the access package; the check
/// constraint in `0024_agent_ports.sql` keeps the two apart.
const KIND_TERMINAL: &str = "terminal";

/// Serialise the limit check for one user.
///
/// Two tabs opened in the same millisecond would otherwise both count three
/// sessions, both decide there is room, and both insert. The lock is
/// transaction-scoped, so it is released by commit or rollback and cannot leak.
pub async fn lock_user(scope: &mut TenantScope<'_>, user_id: Uuid) -> Result<(), StoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(user_id.to_string())
        .execute(scope.connection())
        .await?;
    Ok(())
}

/// Open a session row and return its id.
pub async fn open(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    user_id: Uuid,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sandbox_sessions (environment_id, sandbox_id, user_id, kind)
         VALUES ($1, $2, $3, $4)
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(user_id)
    .bind(KIND_TERMINAL)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// How many terminals are open in one sandbox, across all users.
///
/// `stale_after` is the age at which an unclosed row is treated as abandoned
/// rather than open: a session cannot outlive the idle timeout, so a row older
/// than that is a process that died without cleaning up.
pub async fn count_open_in_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    stale_after: Duration,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM sandbox_sessions
          WHERE environment_id = $1
            AND sandbox_id = $2
            AND kind = $3
            AND closed_at IS NULL
            AND opened_at > now() - ($4::double precision * interval '1 second')",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(KIND_TERMINAL)
    .bind(stale_after.as_secs_f64())
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}

/// How many terminals one user has open, across all sandboxes.
/// Open terminals one person has in one sandbox right now.
pub async fn count_open_for_user_in_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    user_id: Uuid,
    stale_after: Duration,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM sandbox_sessions
          WHERE environment_id = $1
            AND sandbox_id = $2
            AND user_id = $3
            AND kind = $4
            AND closed_at IS NULL
            AND opened_at > now() - ($5::double precision * interval '1 second')",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(user_id)
    .bind(KIND_TERMINAL)
    .bind(stale_after.as_secs_f64())
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}

pub async fn count_open_for_user(
    scope: &mut TenantScope<'_>,
    user_id: Uuid,
    stale_after: Duration,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM sandbox_sessions
          WHERE environment_id = $1
            AND user_id = $2
            AND kind = $3
            AND closed_at IS NULL
            AND opened_at > now() - ($4::double precision * interval '1 second')",
    )
    .bind(scope.environment_id())
    .bind(user_id)
    .bind(KIND_TERMINAL)
    .bind(stale_after.as_secs_f64())
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}

/// Close a session: when, why, and how many bytes.
///
/// The duration is not stored; it is derived from `opened_at` and `closed_at` by
/// whoever reads the row. One value, one clock — a stored duration would be a
/// second truth that can disagree with the timestamps next to it.
pub async fn close(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    reason: &str,
    bytes_in: i64,
    bytes_out: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE sandbox_sessions
            SET closed_at = now(), close_reason = $3, bytes_in = $4, bytes_out = $5
          WHERE environment_id = $1 AND id = $2 AND closed_at IS NULL",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(reason)
    .bind(bytes_in)
    .bind(bytes_out)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Close the sessions that cannot still be alive, and report how many.
///
/// A session is closed by the process that opened it, but `kill -9`, a redeploy,
/// or a power cut cannot. Such a row keeps its quota slot until it ages out of
/// the count, which is why `open` calls this first: the next terminal in that
/// environment frees the ghosts instead of refusing an operator who is looking
/// at an empty Quota tab.
///
/// The reason is `error`, one of the four ADR 0023 names, because whatever
/// happened was not a user closing a shell. `closed_at` is the moment the row
/// was noticed, not the moment the session really ended — the duration of a
/// swept row is "until detection". That is a recorded approximation rather than
/// an invented precision: the alternative is leaving `closed_at` NULL forever,
/// which claims the session is still open.
pub async fn close_stale(
    scope: &mut TenantScope<'_>,
    stale_after: Duration,
) -> Result<u64, StoreError> {
    let result = sqlx::query(
        "UPDATE sandbox_sessions
            SET closed_at = now(), close_reason = 'error'
          WHERE environment_id = $1
            AND kind = $2
            AND closed_at IS NULL
            AND opened_at < now() - ($3::double precision * interval '1 second')",
    )
    .bind(scope.environment_id())
    .bind(KIND_TERMINAL)
    .bind(stale_after.as_secs_f64())
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected())
}
