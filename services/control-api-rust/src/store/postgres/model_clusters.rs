//! Model clusters and their members: the queries behind contract C7.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::model_cluster::{healthy_count, ClusterMember, ClusterStrategy, ModelCluster};
use crate::error::StoreError;

const CLUSTER_COLUMNS: &str =
    "id, name, model_key, strategy, provider_profile_id, created_at";

/// Build a cluster from a row. The endpoint is filled by the service, which
/// knows where the accounting proxy lives; members are loaded separately.
fn row_to_cluster(row: &sqlx::postgres::PgRow) -> Result<ModelCluster, StoreError> {
    let strategy_raw: String = row.try_get("strategy")?;
    let strategy = ClusterStrategy::parse(&strategy_raw)
        .ok_or_else(|| StoreError::UnexpectedData(format!("cluster strategy {strategy_raw}")))?;
    Ok(ModelCluster {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        model_key: row.try_get("model_key")?,
        strategy,
        provider_profile_id: row.try_get("provider_profile_id")?,
        endpoint: String::new(),
        healthy_members: 0,
        members: Vec::new(),
        created_at: row.try_get("created_at")?,
    })
}

/// Fill in the members and the healthy count of a cluster.
async fn with_members(
    scope: &mut TenantScope<'_>,
    mut cluster: ModelCluster,
) -> Result<ModelCluster, StoreError> {
    cluster.members = members(scope, cluster.id).await?;
    cluster.healthy_members = healthy_count(&cluster.members);
    Ok(cluster)
}

/// Every cluster of the scope's data center, with members.
pub async fn list(scope: &mut TenantScope<'_>) -> Result<Vec<ModelCluster>, StoreError> {
    let sql = format!(
        "SELECT {CLUSTER_COLUMNS} FROM model_clusters
          WHERE environment_id = $1 ORDER BY name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::new();
    for row in &rows {
        let cluster = row_to_cluster(row)?;
        out.push(with_members(scope, cluster).await?);
    }
    Ok(out)
}

/// One cluster with its members, or none.
pub async fn get(scope: &mut TenantScope<'_>, id: Uuid) -> Result<Option<ModelCluster>, StoreError> {
    let sql = format!(
        "SELECT {CLUSTER_COLUMNS} FROM model_clusters
          WHERE environment_id = $1 AND id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?;
    match row {
        Some(row) => Ok(Some(with_members(scope, row_to_cluster(&row)?).await?)),
        None => Ok(None),
    }
}

/// Whether the model catalog knows this key.
pub async fn model_exists(scope: &mut TenantScope<'_>, key: &str) -> Result<bool, StoreError> {
    let found: Option<i32> = sqlx::query_scalar("SELECT 1 FROM models WHERE key = $1")
        .bind(key)
        .fetch_optional(scope.connection())
        .await?;
    Ok(found.is_some())
}

/// The model key of one install in this data center, or none.
pub async fn install_model_key(
    scope: &mut TenantScope<'_>,
    install: Uuid,
) -> Result<Option<String>, StoreError> {
    let key = sqlx::query_scalar(
        "SELECT model_key FROM model_installs WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(install)
    .fetch_optional(scope.connection())
    .await?;
    Ok(key)
}

/// Insert a cluster row. The caller chooses the id because the provider
/// profile's endpoint names it before either row exists.
pub async fn create(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    name: &str,
    model_key: &str,
    strategy: ClusterStrategy,
    provider_profile_id: Option<Uuid>,
    created_by: Option<Uuid>,
) -> Result<ModelCluster, StoreError> {
    let sql = format!(
        "INSERT INTO model_clusters
            (id, environment_id, name, model_key, strategy, provider_profile_id, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {CLUSTER_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(id)
        .bind(scope.environment_id())
        .bind(name)
        .bind(model_key)
        .bind(strategy.as_str())
        .bind(provider_profile_id)
        .bind(created_by)
        .fetch_one(scope.connection())
        .await?;
    row_to_cluster(&row)
}

/// Rename a cluster or change its strategy.
pub async fn update(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    name: Option<&str>,
    strategy: Option<ClusterStrategy>,
) -> Result<Option<ModelCluster>, StoreError> {
    let sql = format!(
        "UPDATE model_clusters
            SET name = COALESCE($3, name),
                strategy = COALESCE($4, strategy),
                updated_at = now()
          WHERE environment_id = $1 AND id = $2
      RETURNING {CLUSTER_COLUMNS}"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(name)
        .bind(strategy.map(ClusterStrategy::as_str))
        .fetch_optional(scope.connection())
        .await?;
    match row {
        Some(row) => Ok(Some(with_members(scope, row_to_cluster(&row)?).await?)),
        None => Ok(None),
    }
}

/// Delete a cluster row; its members go with it.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<bool, StoreError> {
    let affected = sqlx::query("DELETE FROM model_clusters WHERE environment_id = $1 AND id = $2")
        .bind(scope.environment_id())
        .bind(id)
        .execute(scope.connection())
        .await?
        .rows_affected();
    Ok(affected > 0)
}

/// Add an install to a cluster, or change its weight when it is already there.
pub async fn add_member(
    scope: &mut TenantScope<'_>,
    cluster: Uuid,
    install: Uuid,
    weight: i32,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO model_cluster_members (cluster_id, install_id, weight)
         SELECT c.id, $3, $4 FROM model_clusters c
          WHERE c.environment_id = $1 AND c.id = $2
         ON CONFLICT (cluster_id, install_id) DO UPDATE SET weight = EXCLUDED.weight",
    )
    .bind(scope.environment_id())
    .bind(cluster)
    .bind(install)
    .bind(weight)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Remove an install from a cluster.
pub async fn remove_member(
    scope: &mut TenantScope<'_>,
    cluster: Uuid,
    install: Uuid,
) -> Result<bool, StoreError> {
    let affected = sqlx::query(
        "DELETE FROM model_cluster_members m
          USING model_clusters c
          WHERE c.id = m.cluster_id AND c.environment_id = $1
            AND m.cluster_id = $2 AND m.install_id = $3",
    )
    .bind(scope.environment_id())
    .bind(cluster)
    .bind(install)
    .execute(scope.connection())
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// The members of one cluster, joined to their node and provider profile.
pub async fn members(scope: &mut TenantScope<'_>, cluster: Uuid) -> Result<Vec<ClusterMember>, StoreError> {
    let rows = sqlx::query(
        "SELECT m.install_id, i.node_id, n.name AS node, i.state, m.weight,
                COALESCE(p.status, 'unknown') AS provider_status,
                COALESCE(p.endpoint, '') AS endpoint
           FROM model_cluster_members m
           JOIN model_clusters c ON c.id = m.cluster_id
           JOIN model_installs i ON i.id = m.install_id
           JOIN nodes n ON n.id = i.node_id
           LEFT JOIN provider_profiles p ON p.id = i.provider_profile_id
          WHERE c.environment_id = $1 AND i.environment_id = $1 AND m.cluster_id = $2
          ORDER BY m.added_at, n.name",
    )
    .bind(scope.environment_id())
    .bind(cluster)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(ClusterMember {
            install_id: row.try_get("install_id")?,
            node_id: row.try_get("node_id")?,
            node: row.try_get("node")?,
            state: row.try_get("state")?,
            provider_status: row.try_get("provider_status")?,
            endpoint: row.try_get("endpoint")?,
            weight: row.try_get("weight")?,
        });
    }
    Ok(out)
}

/// The accounting proxy's view of a cluster (contract C7).
///
/// Takes a scope, not a bare pool: the cluster tables are row-level secured,
/// so the service opens the scope of the appliance's data center first.
pub async fn proxy_view(
    scope: &mut TenantScope<'_>,
    cluster: Uuid,
) -> Result<Option<serde_json::Value>, StoreError> {
    let Some(found) = get(scope, cluster).await? else {
        return Ok(None);
    };
    let members: Vec<serde_json::Value> = found
        .members
        .iter()
        .map(|m| {
            serde_json::json!({
                "install_id": m.install_id,
                "endpoint": m.endpoint,
                "weight": m.weight,
                "healthy": m.state == "installed" && m.provider_status == "reachable",
            })
        })
        .collect();
    Ok(Some(serde_json::json!({
        "id": found.id,
        "strategy": found.strategy.as_str(),
        "model_key": found.model_key,
        "members": members,
    })))
}
