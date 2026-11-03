//! The gateway side of a node: its tunnel state and its egress lock (ADR 0057).

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::node_tunnel::TunnelStatus;
use crate::error::StoreError;

/// The gateway columns of a node: the address the hub dials and the lock the node applies.
///
/// Written inside the transaction that inserts the node, so a node never exists half described
/// (ADR 0057).
pub async fn store_gateway(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    node_id: Uuid,
    public_endpoint: Option<&str>,
    wg_listen_port: Option<i32>,
    egress_lock: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE nodes
            SET public_endpoint = $2, wg_listen_port = $3, egress_lock = $4, updated_at = now()
          WHERE id = $1",
    )
    .bind(node_id)
    .bind(public_endpoint)
    .bind(wg_listen_port)
    .bind(egress_lock)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Whether a node of this name exists anywhere on the installation.
///
/// Node names are the hub's peer comments and the vault key (`node-<name>`), so they are unique
/// across environments, not per tenant.
pub async fn name_taken(pool: &sqlx::PgPool, name: &str) -> Result<bool, StoreError> {
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nodes WHERE name = $1)")
        .bind(name)
        .fetch_one(pool)
        .await?;
    Ok(taken)
}

/// Whether a site exists in this environment, read before an install starts.
pub async fn site_in_environment(
    scope: &mut TenantScope<'_>,
    site_id: Uuid,
) -> Result<bool, StoreError> {
    let found: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sites WHERE id = $1 AND environment_id = $2)")
        .bind(site_id)
        .bind(scope.environment_id())
        .fetch_one(scope.connection())
        .await?;
    Ok(found)
}

/// What a node presents to prove itself and the lock it should apply, in one read.
///
/// Unscoped on purpose twice over: the caller is a node, which has no session and no environment,
/// and `nodes` carries no row-level security (ADR 0034). Only a joined node answers at all.
pub async fn gateway_credentials(
    pool: &sqlx::PgPool,
    node_id: Uuid,
) -> Result<Option<(Option<String>, String)>, StoreError> {
    let row: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT node_token_ref, egress_lock FROM nodes
          WHERE id = $1 AND wg_public_key IS NOT NULL",
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// One joined node as the tunnel refresh needs it: its key, its tenant and what the hub said last.
#[derive(Debug, Clone)]
pub struct TunnelNode {
    /// The node.
    pub id: Uuid,
    /// The tenant the event about this node belongs to.
    pub environment_id: Option<Uuid>,
    /// The key the hub's dump names this peer by.
    pub wg_public_key: String,
    /// `up`, `stale`, `down` or `unknown` from the previous refresh.
    pub tunnel_state: String,
}

/// Every node the hub holds a peer for.
///
/// Unscoped: the refresh runs for the whole installation at once, and `nodes` has no policy.
pub async fn nodes_for_tunnel_refresh(pool: &sqlx::PgPool) -> Result<Vec<TunnelNode>, StoreError> {
    let rows: Vec<(Uuid, Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT id, environment_id, wg_public_key, tunnel_state FROM nodes
          WHERE wg_public_key IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, environment_id, wg_public_key, tunnel_state)| TunnelNode {
            id,
            environment_id,
            wg_public_key,
            tunnel_state,
        })
        .collect())
}

/// Store what a node's tunnel looks like from the hub, without a tenant scope.
///
/// The refresh writes rows for every tenant at once, which a scoped store call cannot express.
pub async fn record_tunnel_system(
    pool: &sqlx::PgPool,
    node_id: Uuid,
    state: &str,
    handshake_at: Option<chrono::DateTime<chrono::Utc>>,
    rx_bytes: i64,
    tx_bytes: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE nodes
            SET tunnel_state = $2, tunnel_handshake_at = $3,
                tunnel_rx_bytes = $4, tunnel_tx_bytes = $5, updated_at = now()
          WHERE id = $1",
    )
    .bind(node_id)
    .bind(state)
    .bind(handshake_at)
    .bind(rx_bytes)
    .bind(tx_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

/// The tunnel of every node in this environment.
pub async fn tunnels(scope: &mut TenantScope<'_>) -> Result<Vec<TunnelStatus>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, name, tunnel_state, tunnel_handshake_at, tunnel_rx_bytes, tunnel_tx_bytes,
                public_endpoint
           FROM nodes
          WHERE environment_id = $1
          ORDER BY is_local DESC, name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let now = chrono::Utc::now();
    let mut out = Vec::new();
    for row in rows {
        let handshake: Option<chrono::DateTime<chrono::Utc>> = row.try_get("tunnel_handshake_at")?;
        out.push(TunnelStatus {
            node_id: row.try_get("id")?,
            node_name: row.try_get("name")?,
            state: row.try_get("tunnel_state")?,
            handshake_age_seconds: handshake.map(|at| (now - at).num_seconds().max(0)),
            rx_bytes: row.try_get("tunnel_rx_bytes")?,
            tx_bytes: row.try_get("tunnel_tx_bytes")?,
            endpoint: row.try_get("public_endpoint")?,
        });
    }
    Ok(out)
}

/// Choose how a node reaches the internet.
pub async fn set_egress_lock(
    scope: &mut TenantScope<'_>,
    node_id: Uuid,
    lock: &str,
) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "UPDATE nodes SET egress_lock = $3, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(node_id)
    .bind(lock)
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// The lock a node should apply, read by the node itself.
pub async fn egress_lock(
    scope: &mut TenantScope<'_>,
    node_id: Uuid,
) -> Result<Option<String>, StoreError> {
    let lock: Option<String> =
        sqlx::query_scalar("SELECT egress_lock FROM nodes WHERE environment_id = $1 AND id = $2")
            .bind(scope.environment_id())
            .bind(node_id)
            .fetch_optional(scope.connection())
            .await?;
    Ok(lock)
}

/// Store what a node's tunnel looks like from the hub.
pub async fn record_tunnel(
    scope: &mut TenantScope<'_>,
    node_id: Uuid,
    state: &str,
    handshake_at: Option<chrono::DateTime<chrono::Utc>>,
    rx_bytes: i64,
    tx_bytes: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE nodes
            SET tunnel_state = $3, tunnel_handshake_at = $4,
                tunnel_rx_bytes = $5, tunnel_tx_bytes = $6, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(node_id)
    .bind(state)
    .bind(handshake_at)
    .bind(rx_bytes)
    .bind(tx_bytes)
    .execute(scope.connection())
    .await?;
    Ok(())
}
