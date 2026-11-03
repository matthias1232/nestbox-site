//! Turning a period of observation into rule drafts in one request (ADR 0053).

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::rule_bulk_service;
use crate::state::AppState;

/// The body of a bulk draft request.
#[derive(Debug, Deserialize)]
pub struct BulkDraftRequest {
    /// Restrict to one sandbox; absent means every visible sandbox.
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    /// Only what was seen since this moment; absent means the last 24 hours.
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
}

/// Draft one allow rule per destination that was denied or only observed.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<BulkDraftRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = rule_bulk_service::drafts_from_observations(
        &state.services,
        &auth.principal,
        body.sandbox_id,
        body.since,
    )
    .await?;
    Ok(Json(result))
}
