//! Node summary, drain and quick deploy (package B07).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::node_ops_service;
use crate::state::AppState;

/// One node with load, sandboxes, installs, images and tunnel.
pub async fn summary(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(node_ops_service::summary(&state.services, &auth.principal, id).await?))
}

/// Move or stop every sandbox of a node, as a job.
pub async fn drain(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let value = node_ops_service::drain(&state.services, &auth.principal, id, body).await?;
    Ok((StatusCode::ACCEPTED, Json(value)))
}

/// Provision or join a node, optionally with a model, as a job.
pub async fn quick(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let value = node_ops_service::quick(&state.services, &auth.principal, body).await?;
    Ok((StatusCode::ACCEPTED, Json(value)))
}
