//! Image assignments and the images x nodes matrix (ADR 0048).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::image_sync_service::{self, Assignment, Matrix};
use crate::state::AppState;

/// What the registry holds, as `name:tag`.
pub async fn registry(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    crate::services::require(&auth.principal, crate::domain::auth::Permission::NodeRead)?;
    let items = image_sync_service::registry_images(&state.config).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// Images x nodes for this data center.
pub async fn matrix(State(state): State<AppState>, auth: Auth) -> Result<Json<Matrix>, ApiError> {
    Ok(Json(image_sync_service::matrix(&state.services, &auth.principal).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignRequest {
    pub image: String,
    /// `environment`, `site` or `node`.
    pub scope_type: String,
    #[serde(default)]
    pub scope_id: Option<Uuid>,
}

/// Assign an image to a scope.
pub async fn assign(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<AssignRequest>,
) -> Result<(StatusCode, Json<Assignment>), ApiError> {
    let made = image_sync_service::assign(&state.services, &auth.principal, &body.image, &body.scope_type, body.scope_id).await?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// Withdraw an assignment.
pub async fn unassign(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    image_sync_service::unassign(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
