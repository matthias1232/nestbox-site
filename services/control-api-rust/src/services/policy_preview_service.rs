//! Policy preview: the compiled OpenShell document and recreate warnings (package B10 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Validate and compile a spec that is not saved yet (contract C11).
pub async fn preview(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B10"))
}

/// The same shape for a stored policy.
pub async fn compiled(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _policy_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B10"))
}
