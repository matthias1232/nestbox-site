//! Who is connected to a sandbox, and closing a shared terminal tab.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::presence::PresenceEntry;
use crate::error::ApiError;
use crate::services::presence_service;
use crate::state::AppState;

/// Everyone connected to this sandbox now.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PresenceEntry>>, ApiError> {
    let entries = presence_service::list(&state.services, &auth.principal, id).await?;
    Ok(Json(entries))
}

/// Close one terminal tab for everyone in the sandbox.
pub async fn close_tab(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, tab)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    presence_service::close_tab_for_all(&state.services, &auth.principal, id, &tab).await?;
    Ok(StatusCode::NO_CONTENT)
}
