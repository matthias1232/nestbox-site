//! The request and response bodies behind a traffic row.
//!
//! `traffic_bodies` (`0022_egress.sql`) is separate from `telemetry_events` on
//! purpose: bodies are large and live 24 h, the hot event table is small and
//! lives longer. A row here is keyed by the event id, is capped at 64 KiB per
//! side by the proxy, and has **already passed the central redactor** before it
//! is inserted (`security::redaction`). That is the only promise a "log
//! everything" switch can honestly make, and it is made in one place.
//!
//! The cap is enforced here as well as at the proxy, because a proxy is not a
//! trusted client: it holds a node token, it does not get to decide how much
//! text lands in the database.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;
use crate::security::redaction;

/// The largest body stored per side. Matches the proxy's buffer limit, so a
/// body that arrives larger than this was truncated rather than captured.
pub const MAX_BODY_BYTES: usize = 65_536;

/// A body row to insert.
///
/// The environment is not a field: the batch's tenant is the scope's tenant, so
/// the two cannot disagree.
pub struct NewBody<'a> {
    pub event_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub request_body: Option<&'a str>,
    pub response_body: Option<&'a str>,
    /// Size of what the proxy saw, after its cap. Kept so a truncated body is
    /// recognisable even when the text looks complete.
    pub request_bytes: i64,
    pub response_bytes: i64,
    pub truncated: bool,
}

/// Insert the bodies of one batch.
///
/// Redaction happens here, at the last point before the database, so a caller
/// cannot store a body verbatim by forgetting a call. A retried batch is a
/// no-op on the primary key `(event_id, ts)` rather than a duplicate row.
pub async fn insert_batch(
    pool: &sqlx::PgPool,
    environment_id: Uuid,
    bodies: &[NewBody<'_>],
) -> Result<u64, StoreError> {
    if bodies.is_empty() {
        return Ok(0);
    }

    let mut scope = TenantScope::begin(pool, environment_id).await?;
    let mut written = 0u64;

    for body in bodies {
        let request_body = body.request_body.map(redaction::redact).map(strip_nul);
        let response_body = body.response_body.map(redaction::redact).map(strip_nul);
        let result = sqlx::query(
            "INSERT INTO traffic_bodies
                (event_id, environment_id, sandbox_id, ts, request_body,
                 response_body, request_bytes, response_bytes, truncated)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (event_id, ts) DO NOTHING",
        )
        .bind(body.event_id)
        .bind(scope.environment_id())
        .bind(body.sandbox_id)
        .bind(body.ts)
        .bind(request_body)
        .bind(response_body)
        .bind(body.request_bytes)
        .bind(body.response_bytes)
        .bind(body.truncated)
        .execute(scope.connection())
        .await?;
        written += result.rows_affected();
    }

    scope.commit().await?;
    Ok(written)
}

/// One stored body, as the console's detail view needs it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredBody {
    pub event_id: Uuid,
    pub ts: DateTime<Utc>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub request_bytes: i64,
    pub response_bytes: i64,
    pub truncated: bool,
}

/// The body behind one event.
///
/// Scoped by the tenant even though `traffic_bodies` has no row-level policy
/// yet: the filter is the control that exists, so it is used rather than
/// assumed.
pub async fn for_event(
    scope: &mut TenantScope<'_>,
    event_id: Uuid,
) -> Result<Option<StoredBody>, StoreError> {
    let row = sqlx::query(
        "SELECT event_id, ts, request_body, response_body, request_bytes,
                response_bytes, truncated
           FROM traffic_bodies
          WHERE event_id = $1 AND environment_id = $2
          ORDER BY ts DESC
          LIMIT 1",
    )
    .bind(event_id)
    .bind(scope.environment_id())
    .fetch_optional(scope.connection())
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    Ok(Some(StoredBody {
        event_id: row.try_get("event_id")?,
        ts: row.try_get("ts")?,
        request_body: row.try_get("request_body")?,
        response_body: row.try_get("response_body")?,
        request_bytes: row.try_get("request_bytes")?,
        response_bytes: row.try_get("response_bytes")?,
        truncated: row.try_get("truncated")?,
    }))
}

/// Delete bodies older than the cut-off, and report how many rows went.
///
/// A `DELETE` rather than a partition drop: `traffic_bodies` has a default
/// partition and day partitions only for the days the maintenance job created,
/// so dropping by name would miss rows. The job calls this hourly with a 24 h
/// cut-off, which keeps the scan small.
pub async fn purge_expired(
    pool: &sqlx::PgPool,
    before: DateTime<Utc>,
) -> Result<u64, StoreError> {
    let result = sqlx::query("DELETE FROM traffic_bodies WHERE ts < $1")
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// PostgreSQL text cannot hold a NUL byte, and a captured body is arbitrary bytes (a TLS
/// handshake read as text, a binary upload). One body with one NUL made the whole batch fail,
/// and the proxy retried it forever; the byte is dropped and the rest of the body is kept.
fn strip_nul(text: String) -> String {
    if text.contains('\u{0}') { text.replace('\u{0}', "") } else { text }
}

#[cfg(test)]
mod nul_tests {
    use super::strip_nul;

    #[test]
    fn a_nul_byte_is_dropped_and_the_rest_kept() {
        assert_eq!(strip_nul("ab\u{0}cd\u{0}".to_string()), "abcd");
        assert_eq!(strip_nul("plain".to_string()), "plain");
    }
}
