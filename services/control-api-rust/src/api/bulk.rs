//! Bulk sandbox actions: `POST /sandboxes/bulk` (package B03).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::bulk_service;
use crate::state::AppState;

/// The body of a bulk action.
#[derive(Debug, Deserialize)]
pub struct BulkRequest {
    pub ids: Vec<Uuid>,
    pub action: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Start the job; answers 202 with the job id.
pub async fn sandboxes(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<BulkRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let value = bulk_service::sandboxes(&state.services, &auth.principal, body.ids, &body.action, body.params).await?;
    Ok((StatusCode::ACCEPTED, Json(value)))
}
