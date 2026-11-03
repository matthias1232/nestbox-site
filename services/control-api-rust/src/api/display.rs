//! The display layout of a desktop sandbox: which monitors exist, how large each
//! one is, where it sits, and whether they are separate screens or one stretched
//! screen (U10a).
//!
//! The route is the console's and the MCP server's one way to the screen layout:
//! `GET` reads what is on the screen, `PUT` replaces it. The rules live in the
//! domain and the execution in the service; this file parses and delegates.
//!
//! A `PUT` body carries `expected_applied_at`, the stamp of the layout the caller
//! read. When it differs from the sandbox's own stamp the answer is a `409
//! display.changed` whose `details` hold the layout that is really there, so the
//! console can redraw instead of overwriting a change an agent just made.

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::display::{AppliedDisplay, DisplayLayout};
use crate::error::ApiError;
use crate::services::display_service;
use crate::state::AppState;

/// The body of `PUT /api/v1/sandboxes/{id}/display`.
#[derive(Debug, Deserialize)]
pub struct SetDisplayRequest {
    /// The layout to apply.
    pub layout: DisplayLayout,
    /// The `applied_at` of the layout the caller last read. Omitted means
    /// "whatever is there may be replaced".
    #[serde(default)]
    pub expected_applied_at: Option<String>,
}

/// Read the display layout of a desktop sandbox.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<AppliedDisplay>, ApiError> {
    let display = display_service::get(&state.services, &auth.principal, id).await?;
    Ok(Json(display))
}

/// Apply a display layout to a desktop sandbox.
pub async fn set(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<SetDisplayRequest>,
) -> Result<Json<AppliedDisplay>, ApiError> {
    let display = display_service::set(
        &state.services,
        &auth.principal,
        id,
        &body.layout,
        body.expected_applied_at.as_deref(),
    )
    .await?;
    Ok(Json(display))
}
