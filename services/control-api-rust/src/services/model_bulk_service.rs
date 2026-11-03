//! One model on many nodes as one job (package B04 builds it).

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Plan and start the bulk install of `key`; returns `{job_id, targets}` (contract C6).
pub async fn install_bulk(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _key: &str,
    _body: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B04"))
}
