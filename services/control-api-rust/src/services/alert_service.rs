//! Alerts: refused destinations awaiting an answer (package B08 builds it).

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::alerts::AlertView;
use crate::domain::auth::Principal;
use crate::error::ApiError;

/// Open alerts in the last `window_minutes`, optionally for one sandbox.
pub async fn list(
    _ctx: &ServiceContext,
    _principal: &Principal,
    _window_minutes: i64,
    _sandbox: Option<Uuid>,
) -> Result<Vec<AlertView>, ApiError> {
    Err(ApiError::internal("not built: B08"))
}

/// Publish an `alert` live event for a refused network or DNS event; a no-op until B08.
pub async fn publish_if_refused(
    _ctx: &ServiceContext,
    _environment_id: Uuid,
    _sandbox_id: Option<Uuid>,
    _alert: &AlertView,
) {
}
