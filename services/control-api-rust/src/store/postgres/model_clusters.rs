//! Model clusters and their members (package B05 fills the queries).

use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::model_cluster::{ClusterMember, ClusterStrategy, ModelCluster};
use crate::error::StoreError;

/// Every cluster of the scope's data center.
pub async fn list(_scope: &mut TenantScope<'_>) -> Result<Vec<ModelCluster>, StoreError> {
    Ok(Vec::new())
}

/// One cluster with its members, or none.
pub async fn get(_scope: &mut TenantScope<'_>, _id: Uuid) -> Result<Option<ModelCluster>, StoreError> {
    Ok(None)
}

/// Insert a cluster row. Not built yet.
pub async fn create(
    _scope: &mut TenantScope<'_>,
    _name: &str,
    _model_key: &str,
    _strategy: ClusterStrategy,
    _created_by: Option<Uuid>,
) -> Result<ModelCluster, StoreError> {
    Err(StoreError::UnexpectedData("not built: B05".into()))
}

/// Rename a cluster or change its strategy.
pub async fn update(
    _scope: &mut TenantScope<'_>,
    _id: Uuid,
    _name: Option<&str>,
    _strategy: Option<ClusterStrategy>,
) -> Result<Option<ModelCluster>, StoreError> {
    Ok(None)
}

/// Delete a cluster row; its members go with it.
pub async fn delete(_scope: &mut TenantScope<'_>, _id: Uuid) -> Result<bool, StoreError> {
    Ok(false)
}

/// Add an install to a cluster.
pub async fn add_member(_scope: &mut TenantScope<'_>, _cluster: Uuid, _install: Uuid) -> Result<(), StoreError> {
    Ok(())
}

/// Remove an install from a cluster.
pub async fn remove_member(_scope: &mut TenantScope<'_>, _cluster: Uuid, _install: Uuid) -> Result<bool, StoreError> {
    Ok(false)
}

/// The members of one cluster.
pub async fn members(_scope: &mut TenantScope<'_>, _cluster: Uuid) -> Result<Vec<ClusterMember>, StoreError> {
    Ok(Vec::new())
}

/// The accounting proxy's view of a cluster (contract C7).
pub async fn proxy_view(_pool: &sqlx::PgPool, _cluster: Uuid) -> Result<Option<serde_json::Value>, StoreError> {
    Ok(None)
}
