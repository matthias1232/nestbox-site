//! Bulk sandbox actions as one job (package B03 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Start one job that applies `action` to every sandbox in `ids`; returns `{job_id, total}`.
pub async fn sandboxes(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _ids: Vec<Uuid>,
    _action: &str,
    _params: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B03"))
}
