//! First-run tours: `PUT /users/me/tours/{tour}` (package F18).

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
