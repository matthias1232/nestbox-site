//! Allow one destination for a while, from the live view.
//!
//! A quick allow is a normal allow rule with an end: it is created by the same
//! service that creates every rule (so validation, cache invalidation and the
//! gateway policy push are the same), and the reconcile pass deletes it when its
//! time is up.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::auth::Principal;
use crate::error::ApiError;
use crate::services::ServiceContext;

/// Body of `POST /sandboxes/{id}/quick-allow`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct QuickAllowRequest {
    pub host: String,
    pub port: Option<i32>,
    /// 15, 60, 1440, or `None` for a permanent rule.
    pub minutes: Option<i64>,
}

/// One expiring allow rule.
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
pub struct QuickAllowRule {
    pub rule_id: Uuid,
    pub host: String,
    pub port: Option<i32>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn not_implemented() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        crate::error::ErrorCode::INTERNAL,
        "Not implemented yet.",
    )
}

/// Create the rule.
pub async fn create(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _sandbox_id: Uuid,
    _request: QuickAllowRequest,
) -> Result<QuickAllowRule, ApiError> {
    Err(not_implemented())
}

/// The expiring rules of one sandbox.
pub async fn list(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _sandbox_id: Uuid,
) -> Result<Vec<QuickAllowRule>, ApiError> {
    Ok(Vec::new())
}

/// Delete the rules whose time is up. Returns how many.
pub async fn expire_due(_ctx: &ServiceContext) -> Result<u64, ApiError> {
    Ok(0)
}
