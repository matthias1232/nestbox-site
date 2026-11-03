//! Model clusters (package B05).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::model_cluster_service;
use crate::state::AppState;

/// Every cluster.
pub async fn list(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(model_cluster_service::list(&state.services, &auth.principal).await?))
}

/// One cluster.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(model_cluster_service::get(&state.services, &auth.principal, id).await?))
}

/// Create a cluster.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let value = model_cluster_service::create(&state.services, &auth.principal, body).await?;
    Ok((StatusCode::CREATED, Json(value)))
}

/// Rename a cluster or change its strategy.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(model_cluster_service::update(&state.services, &auth.principal, id, body).await?))
}

/// Delete a cluster.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    model_cluster_service::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The body of an add-member call.
#[derive(Debug, Deserialize)]
pub struct MemberRequest {
    pub install_id: Uuid,
}

/// Add an install to a cluster.
pub async fn add_member(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<MemberRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(model_cluster_service::add_member(&state.services, &auth.principal, id, body.install_id).await?))
}

/// Remove an install from a cluster.
pub async fn remove_member(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, install_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    model_cluster_service::remove_member(&state.services, &auth.principal, id, install_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The accounting proxy's view of a cluster; authenticated with the bridge token.
pub async fn internal_view(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::accounting::authorised(&headers)?;
    Ok(Json(model_cluster_service::proxy_view(&state.services, id).await?))
}
