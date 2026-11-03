//! The two questions a background revert loop asks before it reaches `policies.rs`.
//!
//! `policies.rs` owns reading and writing one sandbox's egress mode. The loop that
//! switches a sandbox back when its revert timer has passed has no principal and
//! therefore no tenant of its own: it first needs the list of tenants, then, inside
//! one tenant scope, the timers that are due. Both queries live here so no SQL
//! appears in a service file.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;

/// Every environment's id, so a system loop can open one tenant scope per tenant.
///
/// This is the one read that does not filter by `environment_id`, because it is how
/// the caller learns which `environment_id` values exist. It returns ids only, never
/// tenant data, and it is used exactly where `sync_openshell_policies` needs the same
/// list.
pub async fn environment_ids(pool: &sqlx::PgPool) -> Result<Vec<Uuid>, StoreError> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments ORDER BY id")
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// The sandboxes of this environment whose revert timer has passed.
///
/// Each pair is the sandbox and the mode it goes back to; a row without a stored
/// target falls back to `forward_proxy`, which is what an absent mode has always
/// meant. Ordered by the timer so the oldest promise is kept first.
pub async fn due(scope: &mut TenantScope<'_>) -> Result<Vec<(Uuid, String)>, StoreError> {
    let rows = sqlx::query(
        "SELECT sandbox_id, COALESCE(revert_to, 'forward_proxy') AS revert_to
           FROM egress_modes
          WHERE environment_id = $1
            AND revert_at IS NOT NULL
            AND revert_at <= now()
          ORDER BY revert_at",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push((row.try_get("sandbox_id")?, row.try_get("revert_to")?));
    }
    Ok(out)
}
