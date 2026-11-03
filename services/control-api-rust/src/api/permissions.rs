//! Permissions per scope (package B12).

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::permissions::ScopeType;
use crate::error::ApiError;
use crate::services::permission_service;
use crate::state::AppState;

/// The query of the permission list.
#[derive(Debug, Deserialize)]
pub struct ScopeQuery {
    pub scope_type: String,
    pub scope_id: Uuid,
}

/// The body of a role change or removal.
#[derive(Debug, Deserialize)]
pub struct ScopeBody {
    pub user_id: Uuid,
    pub scope_type: String,
    pub scope_id: Uuid,
    #[serde(default)]
    pub role: Option<String>,
}

fn scope_type(value: &str) -> Result<ScopeType, ApiError> {
    ScopeType::parse(value)
        .ok_or_else(|| ApiError::validation("scope_type must be organization, environment or sandbox."))
}

/// Who has which role in a scope.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let scope = scope_type(&query.scope_type)?;
    Ok(Json(permission_service::list(&state.services, &auth.principal, scope, query.scope_id).await?))
}

/// Give a user a role.
pub async fn put(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<ScopeBody>,
) -> Result<StatusCode, ApiError> {
    let scope = scope_type(&body.scope_type)?;
    let role = body.role.as_deref().ok_or_else(|| ApiError::validation("role is required."))?;
    permission_service::set(&state.services, &auth.principal, body.user_id, scope, body.scope_id, role).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a role away.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<ScopeBody>,
) -> Result<StatusCode, ApiError> {
    let scope = scope_type(&body.scope_type)?;
    permission_service::remove(&state.services, &auth.principal, body.user_id, scope, body.scope_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
