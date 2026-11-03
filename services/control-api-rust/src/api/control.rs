//! The one call the Workbox page needs, and the summary the admin list shows.

use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::control_service;
use crate::state::AppState;

/// Everything the Workbox shows for one sandbox.
pub async fn summary(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let summary = control_service::summary(&state.services, &auth.principal, id).await?;
    Ok(Json(summary))
}

/// The per-sandbox summary for every sandbox the caller can see.
pub async fn overview(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let overview = control_service::overview(&state.services, &auth.principal).await?;
    Ok(Json(overview))
}
