//! Model grants: which models a sandbox may use, and the router payload the
//! accounting proxy reads (ADR 0056).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::model_grant::{EffectiveModel, ModelGrant, NewModelGrant};
use crate::error::ApiError;
use crate::services::model_grant_service;
use crate::state::AppState;

/// Filters for the grant list.
#[derive(Debug, Deserialize)]
pub struct GrantQuery {
    /// Only grants of this sandbox.
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    /// Only grants of this user.
    #[serde(default)]
    pub user_id: Option<Uuid>,
}

/// The grants the caller may see.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<GrantQuery>,
) -> Result<Json<Vec<ModelGrant>>, ApiError> {
    let grants = model_grant_service::list(
        &state.services,
        &auth.principal,
        query.sandbox_id,
        query.user_id,
    )
    .await?;
    Ok(Json(grants))
}

/// Grant a model to the environment, a user or one sandbox.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NewModelGrant>,
) -> Result<(StatusCode, Json<ModelGrant>), ApiError> {
    let grant = model_grant_service::create(&state.services, &auth.principal, body).await?;
    Ok((StatusCode::CREATED, Json(grant)))
}

/// Revoke a grant.
pub async fn remove(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    model_grant_service::remove(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The models one sandbox may use, after inheritance.
pub async fn sandbox_models(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<EffectiveModel>>, ApiError> {
    let models = model_grant_service::effective_for_sandbox(&state.services, &auth.principal, id).await?;
    Ok(Json(models))
}

/// The models and upstreams of one sandbox, for the accounting proxy.
///
/// Authenticated like the usage record endpoint: the proxy presents its bridge
/// token, never a user's session.
pub async fn internal_models(
    State(state): State<AppState>,
    Path(sandbox): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::accounting::authorised(&headers)?;
    let routes = model_grant_service::routes_for_accounting(&state.services, &sandbox).await?;
    Ok(Json(routes))
}
