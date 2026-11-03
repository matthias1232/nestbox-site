//! Whether moving a sandbox to another node can work, said before anything stops.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::move_service;
use crate::state::AppState;

/// Which node the move would go to.
#[derive(Debug, Deserialize)]
pub struct PreflightQuery {
    /// The target node's name.
    pub node: String,
}

/// The preflight of a move: blockers, warnings, what is carried over.
pub async fn preflight(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<PreflightQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let answer =
        move_service::preflight(&state.services, &auth.principal, id, &query.node).await?;
    Ok(Json(answer))
}
