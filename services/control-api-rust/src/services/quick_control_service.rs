//! One-click controls for the end user (package B11 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Apply one quick control and answer the sandbox's control summary (contract C12).
pub async fn set(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _sandbox_id: Uuid,
    _what: &str,
    _value: serde_json::Value,
    _minutes: Option<i32>,
) -> Result<serde_json::Value, ApiError> {
    Err(ApiError::internal("not built: B11"))
}
