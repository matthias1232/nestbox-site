//! The sign-in mode of each agent in one sandbox, and restarting an agent
//! (ADR 0054).

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::agent_auth::{AgentAuthMode, AgentKind, AgentRuntimeState, RestartPolicy};
use crate::error::ApiError;
use crate::services::agent_auth_service;
use crate::state::AppState;

/// Every switchable agent of a sandbox with its mode and what it is doing.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AgentRuntimeState>>, ApiError> {
    let agents = agent_auth_service::list(&state.services, &auth.principal, id).await?;
    Ok(Json(agents))
}

/// The same answer under the name the console's activity view uses.
pub async fn activity(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<AgentRuntimeState>>, ApiError> {
    let agents = agent_auth_service::list(&state.services, &auth.principal, id).await?;
    Ok(Json(agents))
}

/// The body of a mode change.
#[derive(Debug, Deserialize)]
pub struct AgentAuthRequest {
    /// `subscription` or `gateway`.
    pub mode: String,
    /// `idle` (default), `now` or `later`.
    #[serde(default)]
    pub restart: Option<String>,
}

/// Switch one agent between the own subscription and the gateway.
pub async fn set(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, agent)): Path<(Uuid, String)>,
    Json(body): Json<AgentAuthRequest>,
) -> Result<Json<Vec<AgentRuntimeState>>, ApiError> {
    let agent = AgentKind::parse(&agent)
        .ok_or_else(|| ApiError::validation("An agent is claude, codex or opencode."))?;
    let mode = AgentAuthMode::parse(&body.mode)
        .ok_or_else(|| ApiError::validation("A sign-in mode is subscription or gateway."))?;
    let restart = match body.restart.as_deref() {
        None => RestartPolicy::Idle,
        Some(value) => RestartPolicy::parse(value)
            .ok_or_else(|| ApiError::validation("A restart policy is idle, now or later."))?,
    };
    let agents = agent_auth_service::set(&state.services, &auth.principal, id, agent, mode, restart).await?;
    Ok(Json(agents))
}

/// The body of a restart request.
#[derive(Debug, Deserialize)]
pub struct RestartRequest {
    /// `idle` (default), `now` or `later`.
    #[serde(default)]
    pub when: Option<String>,
}

/// Restart one agent without changing its mode.
pub async fn restart(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, agent)): Path<(Uuid, String)>,
    Json(body): Json<RestartRequest>,
) -> Result<Json<Vec<AgentRuntimeState>>, ApiError> {
    let agent = AgentKind::parse(&agent)
        .ok_or_else(|| ApiError::validation("An agent is claude, codex or opencode."))?;
    let when = match body.when.as_deref() {
        None => RestartPolicy::Idle,
        Some(value) => RestartPolicy::parse(value)
            .ok_or_else(|| ApiError::validation("A restart policy is idle, now or later."))?,
    };
    let agents = agent_auth_service::restart(&state.services, &auth.principal, id, agent, when).await?;
    Ok(Json(agents))
}
