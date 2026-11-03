//! Warming a model so the next request is fast (ADR 0056).

use axum::Json;
use axum::extract::{Path, State};

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::model_readiness_service;
use crate::state::AppState;

/// Start the model if needed and ask it one tiny question.
pub async fn warm(
    State(state): State<AppState>,
    auth: Auth,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = model_readiness_service::warm(&state.services, &auth.principal, &key).await?;
    Ok(Json(result))
}
