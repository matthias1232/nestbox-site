//! Flat rows for the inventory tree (package B01 fills the queries).

use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::inventory::{InstallRow, PlacementRow, UserRow};
use crate::error::StoreError;

/// Every sandbox of the scope's data center with its site, node and owner.
pub async fn placement_rows(_scope: &mut TenantScope<'_>) -> Result<Vec<PlacementRow>, StoreError> {
    Ok(Vec::new())
}

/// The people of the scope's data center, for the workloads view.
pub async fn user_rows(_scope: &mut TenantScope<'_>) -> Result<Vec<UserRow>, StoreError> {
    Ok(Vec::new())
}

/// Model installs of the scope's data center, for the models view.
pub async fn install_rows(
    _scope: &mut TenantScope<'_>,
    _model_key: Option<Uuid>,
) -> Result<Vec<InstallRow>, StoreError> {
    Ok(Vec::new())
}
