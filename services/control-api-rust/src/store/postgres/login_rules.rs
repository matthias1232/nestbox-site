//! The firewall rules that let an agent reach its vendor's sign-in hosts.
//!
//! When an agent signs in with the user's own account, the identity hosts of its
//! vendor have to be reachable, and the default policy rightly does not allow
//! them. They are written as ordinary per-sandbox network rules named
//! `Sign-in (<agent>): <host>`, so an operator sees them next to every other
//! rule and can delete one if it is not wanted (ADR 0054).
//!
//! The rules belong to the mode, not to the creation: switching an agent back to
//! the gateway removes them again. That is why they carry a name the sync can
//! recognise instead of an id somebody has to remember.

use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::policy::{matchers_from_flat, RuleProtocol};
use crate::error::StoreError;

/// The port every sign-in host is reached on.
const SIGN_IN_PORT: i32 = 443;

/// The name prefix of every rule this module owns.
const SIGN_IN_PREFIX: &str = "Sign-in";

/// Delete the sign-in rules of one sandbox and report how many went.
///
/// Both spellings are removed: the per-agent form this module writes and the
/// older per-host `Sign-in: <host>` form that `sandbox_service::create` wrote for
/// a sandbox created with `login: own_account`. The older form would otherwise
/// keep the vendor hosts open after the last agent switched to the gateway,
/// which is exactly the access the mode change is meant to take away.
pub async fn delete_for_sandbox(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<u64, StoreError> {
    let affected = sqlx::query(
        "DELETE FROM network_rules
          WHERE environment_id = $1 AND sandbox_id = $2
            AND (name LIKE $3 OR name LIKE $4)",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(format!("{SIGN_IN_PREFIX} (%"))
    .bind(format!("{SIGN_IN_PREFIX}:%"))
    .execute(scope.connection())
    .await?
    .rows_affected();
    Ok(affected)
}

/// Allow one host on 443 for one sandbox.
///
/// The columns are the ones `sandbox_service::create` writes for its sign-in
/// rules — allow, port 443, `rest`, priority 100, enforced — because a rule built
/// two different ways is a rule an operator has to read twice. `matchers` is
/// filled from the flat fields, because that column is what the decider
/// evaluates and the flat fields are only kept for older readers.
pub async fn insert(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    name: &str,
    host: &str,
) -> Result<(), StoreError> {
    let matchers = matchers_from_flat(host, Some(SIGN_IN_PORT), RuleProtocol::Rest, &[], &[]);
    let matchers = serde_json::to_value(matchers)
        .map_err(|error| StoreError::UnexpectedData(error.to_string()))?;
    sqlx::query(
        "INSERT INTO network_rules
            (environment_id, sandbox_id, name, action, host, port, protocol, matchers,
             enforcement, priority, enabled)
         VALUES ($1, $2, $3, 'allow', $4, $5, 'rest', $6, 'enforced', 100, true)",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(name)
    .bind(host)
    .bind(SIGN_IN_PORT)
    .bind(matchers)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Every environment, oldest first.
///
/// The background loop that finishes waiting restarts has no request and
/// therefore no tenant; it walks the tenants one at a time and opens a scope for
/// each. The same query exists inline in two other stores; it lives here because
/// this module's loop needs it and a store is the only place SQL may appear.
pub async fn environment_ids(pool: &sqlx::PgPool) -> Result<Vec<Uuid>, StoreError> {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM environments ORDER BY created_at, id")
            .fetch_all(pool)
            .await?;
    Ok(ids)
}
