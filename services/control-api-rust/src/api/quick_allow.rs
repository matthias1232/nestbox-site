//! Allow one destination for a while, from the live view.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::quick_allow::{self, QuickAllowRequest, QuickAllowRule};
use crate::state::AppState;

/// Create the rule.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(sandbox_id): Path<Uuid>,
    Json(body): Json<QuickAllowRequest>,
) -> Result<(StatusCode, Json<QuickAllowRule>), ApiError> {
    let rule = quick_allow::create(&state.services, &auth.principal, sandbox_id, body).await?;
    Ok((StatusCode::CREATED, Json(rule)))
}

/// The expiring rules of one sandbox.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(sandbox_id): Path<Uuid>,
) -> Result<Json<Vec<QuickAllowRule>>, ApiError> {
    Ok(Json(
        quick_allow::list(&state.services, &auth.principal, sandbox_id).await?,
    ))
}
