//! Node summary, drain and quick deploy (package B07 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// One node with load, sandboxes, installs, images and tunnel (contract C8).
pub async fn summary(_ctx: &ServiceContext, _principal: &Principal, _id: Uuid) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B07"))
}

/// Start a drain job; returns `{job_id}`.
pub async fn drain(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _id: Uuid,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B07"))
}

/// Start a quick-deploy job; returns `{job_id, join_command}`.
pub async fn quick(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B07"))
}
