//! First-run tours: `PUT /users/me/tours/{tour}` and `GET /users/me/tours`.

use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::tour_service;
use crate::state::AppState;

/// Mark a tour as seen for the signed-in user.
pub async fn mark_seen(
    State(state): State<AppState>,
    auth: Auth,
    Path(tour): Path<String>,
) -> Result<StatusCode, ApiError> {
    tour_service::mark_seen(&state.services, &auth.principal, &tour).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The tours the signed-in user has finished, as `{ "items": ["shell-v1"] }`.
///
/// Wave 0 did not register this route; `api/mod.rs` needs `GET /api/v1/users/me/tours`.
pub async fn list_seen(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    let items = tour_service::seen(&state.services, &auth.principal).await?;
    Ok(axum::Json(serde_json::json!({ "items": items })))
}
