//! A refusal the broker reports: it stopped a request without a finding.
//!
//! The content check could not answer, or the body was larger than it reads. That
//! is a decision about a person's request, so it is recorded where every other
//! finding is — otherwise the chat says "sent" and nothing explains the 503.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

/// What the broker reports.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RefusalBody {
    pub sandbox_id: Uuid,
    /// `unavailable` or `too_large`.
    pub kind: String,
    pub destination: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub request_ref: Option<String>,
    pub model: Option<String>,
}

/// Record one refusal.
pub async fn refusal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RefusalBody>,
) -> Result<StatusCode, ApiError> {
    crate::api::accounting::ingest_authorised(&state, &headers).await?;
    // The broker knows the sandbox id; the environment is what the row needs, and
    // an id that no sandbox has is a 404 rather than a row in the wrong tenant.
    let environment =
        crate::store::postgres::egress::sandbox_environment(&state.services.pool, body.sandbox_id)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| {
                ApiError::not_found(
                    crate::error::ErrorCode::NOT_FOUND,
                    "No such sandbox in this environment.",
                )
            })?;
    crate::services::privacy_service::record_refusal(
        &state.services,
        environment,
        body.sandbox_id,
        &body.kind,
        body.destination.as_deref(),
        body.method.as_deref(),
        body.path.as_deref(),
        body.request_ref.as_deref(),
        body.model.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
