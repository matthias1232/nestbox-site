//! Model clusters: create, change, delete, membership (package B05 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Every cluster of the caller's data center.
pub async fn list(_ctx: &ServiceContext, _principal: &Principal) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// One cluster with its members.
pub async fn get(_ctx: &ServiceContext, _principal: &Principal, _id: Uuid) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// Create a cluster and its provider profile.
pub async fn create(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// Change a cluster's name or strategy.
pub async fn update(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _id: Uuid,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// Delete a cluster and its provider profile.
pub async fn delete(_ctx: &ServiceContext, _principal: &Principal, _id: Uuid) -> Result<(), ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// Add an install to a cluster.
pub async fn add_member(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _id: Uuid,
    _install_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// Remove an install from a cluster.
pub async fn remove_member(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _id: Uuid,
    _install_id: Uuid,
) -> Result<(), ApiError> {
    Err(ApiError::internal("not built: B05"))
}

/// The accounting proxy's view of a cluster (node token, no principal).
pub async fn proxy_view(_ctx: &ServiceContext, _id: Uuid) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B05"))
}
