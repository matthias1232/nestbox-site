//! Roles per scope (package B12 fills the queries).

use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::permissions::{PermissionRow, ScopeType};
use crate::error::StoreError;

/// Everyone with a role in the scope's data center.
pub async fn list_environment(_scope: &mut TenantScope<'_>) -> Result<Vec<PermissionRow>, StoreError> {
    Ok(Vec::new())
}

/// Everyone granted access to one sandbox.
pub async fn list_sandbox(_scope: &mut TenantScope<'_>, _sandbox: Uuid) -> Result<Vec<PermissionRow>, StoreError> {
    Ok(Vec::new())
}

/// Everyone who reaches the data center through the organisation tree.
pub async fn list_organization(_scope: &mut TenantScope<'_>) -> Result<Vec<PermissionRow>, StoreError> {
    Ok(Vec::new())
}

/// Write one role.
pub async fn set_scope(
    _scope: &mut TenantScope<'_>,
    _user: Uuid,
    _scope_type: ScopeType,
    _scope_id: Uuid,
    _role: &str,
) -> Result<(), StoreError> {
    Ok(())
}

/// Remove one role.
pub async fn remove_scope(
    _scope: &mut TenantScope<'_>,
    _user: Uuid,
    _scope_type: ScopeType,
    _scope_id: Uuid,
) -> Result<bool, StoreError> {
    Ok(false)
}
