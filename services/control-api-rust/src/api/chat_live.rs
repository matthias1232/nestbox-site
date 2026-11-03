//! The live report of a model request, from the broker or the accounting proxy.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::chat_live::{self, LiveChunk};
use crate::state::AppState;

/// Accept one live report. Nothing is stored.
pub async fn accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(chunk): Json<LiveChunk>,
) -> Result<StatusCode, ApiError> {
    crate::api::accounting::ingest_authorised(&state, &headers).await?;
    chat_live::accept(&state.services, chunk).await?;
    Ok(StatusCode::NO_CONTENT)
}
