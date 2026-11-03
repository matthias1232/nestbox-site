//! Policy preview and compiled documents (package B10).

use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::policy_preview_service;
use crate::state::AppState;

/// Compile a spec that is not saved yet.
pub async fn preview(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(policy_preview_service::preview(&state.services, &auth.principal, body).await?))
}

/// The compiled document of a stored policy.
pub async fn compiled(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(policy_preview_service::compiled(&state.services, &auth.principal, id).await?))
}
