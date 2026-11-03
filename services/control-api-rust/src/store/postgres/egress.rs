//! The compiled policy set the proxy enforces, and the decisions it recorded.
//!
//! Three tables (`0022_egress.sql`):
//!
//! * `policy_compilations` — the hash of the rule set last compiled for an
//!   environment, so "deployed" is a comparison rather than an opinion:
//!   `nodes.policy_hash_reported == policy_compilations.hash`;
//! * `policy_checks` — one row per decision the proxy asked for, partitioned by
//!   day, 30 d retention;
//! * `traffic_windows` — the open "allow everything" windows, with a ceiling
//!   (`max_duration_minutes`) so one cannot be left open by accident.
//!
//! The read path for a decision is one query: the enabled rules for the
//! environment plus the open windows, so the decider never asks twice for the
//! same request.
//!
//! Two lookups here are deliberately not tenant-scoped, and the reason is worth
//! stating once:
//!
//! * `nodes` is a global table (a machine is shared by every environment), so
//!   the node token locator and the reported policy hash are read and written
//!   without a scope;
//! * `sandbox_scope` has to answer "which environment is this sandbox in"
//!   *before* a scope can be set. It walks the environments table — which has no
//!   row-level policy, by design — and asks each candidate through a scoped
//!   transaction, so row-level security stays in force. No privileged role, no
//!   bypass.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::egress::{OpenWindow, WindowChannel, WindowMode};
use crate::error::StoreError;

/// Which environment a sandbox belongs to, and which node runs it.
///
/// The decision function needs the environment to load the right rule set, and
/// the node to refuse a request about somebody else's sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxScope {
    pub environment_id: Uuid,
    pub node_id: Uuid,
}

/// The sandbox with this name, if one exists in any environment.
///
/// Used to turn "the container at 172.30.0.5" into a sandbox row: the runner
/// knows addresses and names, the database knows ids. Names are unique within an
/// environment, and the appliance has one, so the first match is the answer.
/// The environment a sandbox belongs to, by id.
///
/// The egress broker knows the sandbox id (Envoy copies it out of the policy
/// answer) and needs the tenant to write an incident. A missing row is `None`,
/// not an error: a sandbox deleted a moment ago is the normal race.
pub async fn sandbox_environment(
    pool: &sqlx::PgPool,
    sandbox_id: Uuid,
) -> Result<Option<Uuid>, StoreError> {
    // Through the function, not the table: `sandboxes` is under tenant
    // isolation and this process has no tenant yet — the whole question is which
    // one it is. `nb_sandbox_environment` (migration 0080) answers that one
    // question and nothing else.
    let found: Option<Uuid> = sqlx::query_scalar("SELECT nb_sandbox_environment($1)")
        .bind(sandbox_id)
        .fetch_one(pool)
        .await?;
    Ok(found)
}

pub async fn sandbox_id_by_name(
    pool: &sqlx::PgPool,
    name: &str,
) -> Result<Option<Uuid>, StoreError> {
    let candidates: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM environments ORDER BY created_at, id")
            .fetch_all(pool)
            .await?;
    for environment_id in candidates {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('app.environment_id', $1, true)")
            .bind(environment_id.to_string())
            .execute(&mut *tx)
            .await?;
        let found: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM sandboxes
              WHERE environment_id = $1 AND name = $2 AND status <> 'deleted'",
        )
        .bind(environment_id)
        .bind(name)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

/// Resolve the tenant and the node of one sandbox.
///
/// This is the one read a node-authenticated decision cannot avoid doing before
/// it has a tenant scope: the environment is a column *on* the sandbox row.
/// Row-level security would hide an unscoped read from the application role, so
/// the lookup walks the candidate environments and asks each within its own
/// scope. The environments table is tiny (one row on the appliance) and the
/// sandbox id is a primary key, so the cost is a few index probes — and the
/// alternative, a privileged read, is exactly the shortcut that makes a tenant
/// filter optional.
pub async fn sandbox_scope(
    pool: &sqlx::PgPool,
    sandbox_id: Uuid,
) -> Result<Option<SandboxScope>, StoreError> {
    let candidates: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM environments ORDER BY created_at, id")
            .fetch_all(pool)
            .await?;

    for environment_id in candidates {
        let mut tx = pool.begin().await?;
        sqlx::query("SELECT set_config('app.environment_id', $1, true)")
            .bind(environment_id.to_string())
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "SELECT node_id FROM sandboxes
              WHERE environment_id = $1 AND id = $2 AND status <> 'deleted'",
        )
        .bind(environment_id)
        .bind(sandbox_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = row {
            let node_id: Uuid = row.try_get("node_id")?;
            tx.commit().await?;
            return Ok(Some(SandboxScope {
                environment_id,
                node_id,
            }));
        }
        tx.rollback().await?;
    }

    Ok(None)
}

/// The windows that cover traffic right now.
///
/// `now()` is the database's clock, so a window opened by the console and a
/// decision taken by the proxy agree even if the process clock drifts.
pub async fn open_windows(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
) -> Result<Vec<OpenWindow>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, sandbox_id, expires_at, reason, mode, channel
           FROM traffic_windows
          WHERE environment_id = $1
            AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > now())
            AND (sandbox_id IS NULL OR sandbox_id = $2)
          ORDER BY expires_at DESC NULLS FIRST",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mode: String = row.try_get("mode")?;
        let channel: String = row.try_get("channel")?;
        out.push(OpenWindow {
            id: row.try_get("id")?,
            sandbox_id: row.try_get("sandbox_id")?,
            expires_at: row.try_get("expires_at")?,
            reason: row.try_get("reason")?,
            // A word this build does not know is the conservative default: a
            // bypass would be a wider permission than the row asked for.
            mode: WindowMode::parse(&mode).unwrap_or_default(),
            channel: WindowChannel::parse(&channel).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// One decision, ready to be written.
///
/// A struct rather than nine arguments: the row has more columns than a
/// signature can carry without one of them being swapped for another.
pub struct NewDecision<'a> {
    pub sandbox_id: Option<Uuid>,
    pub node_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub host: &'a str,
    pub port: i32,
    pub protocol: &'a str,
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub decision: &'a str,
    pub reason: &'a str,
    pub rule_id: Option<Uuid>,
    pub window_id: Option<Uuid>,
    pub cache_hit: bool,
}

/// Record one decision in `policy_checks`.
///
/// The row is the evidence the Traffic and Policies screens read; a decision
/// that is not written is a decision the operator cannot see, so this is not
/// best-effort inside the service: it runs in the same transaction as the read.
pub async fn record_decision(
    scope: &mut TenantScope<'_>,
    decision: NewDecision<'_>,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO policy_checks
            (environment_id, sandbox_id, node_id, ts, host, port, protocol, method, path,
             decision, reason, rule_id, window_id, cache_hit)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(decision.sandbox_id)
    .bind(decision.node_id)
    .bind(decision.ts)
    .bind(decision.host)
    .bind(decision.port)
    .bind(decision.protocol)
    .bind(decision.method)
    .bind(decision.path)
    .bind(decision.decision)
    .bind(decision.reason)
    .bind(decision.rule_id)
    .bind(decision.window_id)
    .bind(decision.cache_hit)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// Remember the hash of the compiled rule set.
///
/// Returns `true` only when the hash changed, so `compiled_at` keeps meaning
/// "when the rule set last changed" instead of "when this request happened".
/// The counts are refreshed on every call, because how many rules and windows a
/// decision saw is a fact about *now*, not about the last change: a stale count
/// next to a fresh hash reads as a lie.
///
/// The previous hash is read first rather than compared inside the upsert. That
/// version was written and disproved by a two-statement test: in an
/// `ON CONFLICT DO UPDATE`, the target alias in the `SET` expression does not
/// reliably name the old row, so `compiled.hash <> EXCLUDED.hash` was true even
/// when the hashes matched and `compiled_at` moved on every request. A read and
/// a branch have no such ambiguity.
pub async fn record_compilation(
    scope: &mut TenantScope<'_>,
    hash: &str,
    rule_count: i64,
    window_count: i64,
) -> Result<bool, StoreError> {
    let previous: Option<String> =
        sqlx::query_scalar("SELECT hash FROM policy_compilations WHERE environment_id = $1")
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await?;
    let changed = previous.as_deref() != Some(hash);

    if changed {
        sqlx::query(
            "INSERT INTO policy_compilations
                 (environment_id, hash, rule_count, window_count, compiled_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (environment_id) DO UPDATE
                 SET hash = EXCLUDED.hash,
                     rule_count = EXCLUDED.rule_count,
                     window_count = EXCLUDED.window_count,
                     compiled_at = now()",
        )
        .bind(scope.environment_id())
        .bind(hash)
        .bind(rule_count)
        .bind(window_count)
        .execute(scope.connection())
        .await?;
    } else {
        sqlx::query(
            "UPDATE policy_compilations
                SET rule_count = $2, window_count = $3
              WHERE environment_id = $1",
        )
        .bind(scope.environment_id())
        .bind(rule_count)
        .bind(window_count)
        .execute(scope.connection())
        .await?;
    }

    Ok(changed)
}

/// One node's view of the rule set.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeDeployment {
    pub node_id: Uuid,
    pub node_name: String,
    /// The hash the node's proxy last announced, if it ever did.
    pub reported_hash: Option<String>,
    pub reported_at: Option<DateTime<Utc>>,
    /// `reported_hash == compiled_hash`. A node that never reported is not
    /// deployed, because "never told us" and "told us the right thing" are
    /// different facts.
    pub deployed: bool,
}

/// The compiled hash and what each node that runs this environment reports.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyDeployment {
    pub environment_id: Uuid,
    /// `None` until a decision has been taken in this environment, because
    /// that is the moment the rule set is compiled.
    pub compiled_hash: Option<String>,
    pub compiled_at: Option<DateTime<Utc>>,
    pub nodes: Vec<NodeDeployment>,
}

/// Whether each node enforces the current rule set.
///
/// The console shows a green or amber badge from this; the comparison is here
/// rather than in the browser so the two hashes cannot be compared by a client
/// that read them at different times.
pub async fn deployment(scope: &mut TenantScope<'_>) -> Result<PolicyDeployment, StoreError> {
    let compiled =
        sqlx::query("SELECT hash, compiled_at FROM policy_compilations WHERE environment_id = $1")
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await?;

    let (compiled_hash, compiled_at): (Option<String>, Option<DateTime<Utc>>) = match compiled {
        Some(row) => (
            Some(row.try_get("hash")?),
            Some(row.try_get("compiled_at")?),
        ),
        None => (None, None),
    };

    let rows = sqlx::query(
        "SELECT n.id, n.name, n.policy_hash_reported, n.policy_hash_reported_at
           FROM nodes n
          WHERE n.id IN (
                SELECT node_id FROM sandboxes
                 WHERE environment_id = $1 AND status <> 'deleted'
          )
          ORDER BY n.name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let reported_hash: Option<String> = row.try_get("policy_hash_reported")?;
        let deployed = match (&compiled_hash, &reported_hash) {
            (Some(compiled), Some(reported)) => compiled == reported,
            _ => false,
        };
        nodes.push(NodeDeployment {
            node_id: row.try_get("id")?,
            node_name: row.try_get("name")?,
            reported_hash,
            reported_at: row.try_get("policy_hash_reported_at")?,
            deployed,
        });
    }

    Ok(PolicyDeployment {
        environment_id: scope.environment_id(),
        compiled_hash,
        compiled_at,
        nodes,
    })
}

/// The `file://` locator of one node's bearer token.
///
/// A locator, never a value: the token itself lives in a 0600 file that the API
/// reads through `security::secrets` (ADR 0005, ADR 0016).
pub async fn node_token_locator(
    pool: &sqlx::PgPool,
    node_id: Uuid,
) -> Result<Option<String>, StoreError> {
    let locator: Option<String> =
        sqlx::query_scalar("SELECT node_token_ref FROM nodes WHERE id = $1")
            .bind(node_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    Ok(locator)
}

/// Record which rule set a node's proxy says it enforces.
///
/// The proxy reports its own hash; the control plane must not report it on the
/// proxy's behalf, or "deployed" would always be true. Only a change is written,
/// so `policy_hash_reported_at` keeps meaning "when the node last changed what
/// it enforces".
pub async fn record_reported_hash(
    scope: &mut TenantScope<'_>,
    node_id: Uuid,
    hash: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE nodes
            SET policy_hash_reported = $2, policy_hash_reported_at = now()
          WHERE id = $1 AND policy_hash_reported IS DISTINCT FROM $2",
    )
    .bind(node_id)
    .bind(hash)
    .execute(scope.connection())
    .await?;
    Ok(())
}
