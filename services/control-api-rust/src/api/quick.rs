//! Quick controls for the end user: `PUT /sandboxes/{id}/quick` (package B11).

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::quick_control_service;
use crate::state::AppState;

/// The body of a quick control.
#[derive(Debug, Deserialize)]
pub struct QuickRequest {
    /// `allow_all`, `egress_mode`, `dns`, `internet`, `mcp` or `recording`.
    pub what: String,
    pub value: serde_json::Value,
    #[serde(default)]
    pub minutes: Option<i32>,
}

/// Apply one control; answers the sandbox's control summary.
pub async fn set(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<QuickRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let value = quick_control_service::set(&state.services, &auth.principal, id, &body.what, body.value, body.minutes).await?;
    Ok(Json(value))
}
