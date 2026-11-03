//! DLP exceptions: list, create from one incident, delete.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::privacy_exception::PrivacyException;
use crate::error::ApiError;
use crate::services::privacy_exceptions::{self, NewExceptionRequest};
use crate::state::AppState;

/// The list's filters.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
}

/// The exceptions a principal may see.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<PrivacyException>>, ApiError> {
    Ok(Json(
        privacy_exceptions::list(&state.services, &auth.principal, query.sandbox_id).await?,
    ))
}

/// Create an exception from one incident.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<NewExceptionRequest>,
) -> Result<(StatusCode, Json<PrivacyException>), ApiError> {
    let exception = privacy_exceptions::create(&state.services, &auth.principal, body).await?;
    Ok((StatusCode::CREATED, Json(exception)))
}

/// Remove one exception.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    privacy_exceptions::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
