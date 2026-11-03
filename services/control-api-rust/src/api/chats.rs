//! Chats: the turn list, its groups, one turn with its findings, and the audit of
//! a live view.
//!
//! The handlers parse and delegate; the rules live in `services::chat_service`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::chat_service::{self, ChatGroup, ChatTurn, ChatTurnDetail, TurnQuery};
use crate::state::AppState;

/// The turn list.
pub async fn turns(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<TurnQuery>,
) -> Result<Json<Vec<ChatTurn>>, ApiError> {
    Ok(Json(chat_service::turns(&state.services, &auth.principal, query).await?))
}

/// The same rows, grouped by session, model, provider, day, sandbox or owner.
pub async fn groups(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<TurnQuery>,
) -> Result<Json<Vec<ChatGroup>>, ApiError> {
    Ok(Json(chat_service::groups(&state.services, &auth.principal, query).await?))
}

/// One turn with the findings of its request.
pub async fn turn(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<ChatTurnDetail>, ApiError> {
    Ok(Json(chat_service::turn(&state.services, &auth.principal, id).await?))
}

/// What an administrator was watching, for the audit trail.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LiveWatchBody {
    /// `requests`, `all-chats` or `thread`; at most 50 characters.
    pub what: String,
}

/// Record that a live view was opened.
pub async fn live_watch(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<LiveWatchBody>,
) -> Result<StatusCode, ApiError> {
    chat_service::record_live_watch(&state.services, &auth.principal, &body.what).await?;
    Ok(StatusCode::NO_CONTENT)
}
