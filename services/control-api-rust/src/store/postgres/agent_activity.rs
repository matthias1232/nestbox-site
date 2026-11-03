//! The telemetry evidence that a sandbox talked to a model recently.
//!
//! Kept here, not in the service, because it is a query over the append-only
//! event log; the service only asks the question (ADR 0054).

use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;

/// Whether the sandbox produced an inference or network event in the last 30
/// seconds.
///
/// The partition key of `telemetry_events` is `ts`, so the window filter is also
/// what keeps the query on the current partition.
pub async fn recent_model_traffic(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let seen: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM telemetry_events
              WHERE environment_id = $1
                AND sandbox_id = $2
                AND kind IN ('inference', 'network')
                AND ts > now() - interval '30 seconds'
         )",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_one(scope.connection())
    .await?;
    Ok(seen)
}
