//! Connecting a machine by address, the tunnel state per node, and the node's
//! egress lock (ADR 0057).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::node_tunnel::{ConnectNodeRequest, TunnelStatus};
use crate::error::ApiError;
use crate::services::node_connect_service;
use crate::state::AppState;

/// Install a machine as a node over SSH; answers 202 while the work runs.
pub async fn connect(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<ConnectNodeRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let answer =
        node_connect_service::connect(&state.services, &state.config, &auth.principal, body).await?;
    Ok((StatusCode::ACCEPTED, Json(answer)))
}

/// The tunnel of every node the caller can see.
pub async fn tunnels(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Vec<TunnelStatus>>, ApiError> {
    let tunnels = node_connect_service::tunnels(&state.services, &auth.principal).await?;
    Ok(Json(tunnels))
}

/// The body of an egress lock change.
#[derive(Debug, Deserialize)]
pub struct LockRequest {
    /// `strict` or `exit_local`.
    pub lock: String,
}

/// Choose how a node reaches the internet.
pub async fn set_egress_lock(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<LockRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let answer =
        node_connect_service::set_egress_lock(&state.services, &auth.principal, id, &body.lock).await?;
    Ok(Json(answer))
}

/// What the node itself reads with its token: the lock it should apply.
pub async fn node_egress_lock(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let missing = || ApiError::unauthenticated("A node identifies itself with its id and token.");
    let node_id = text("x-nestbox-node")
        .and_then(|value| Uuid::parse_str(&value).ok())
        .ok_or_else(missing)?;
    let token = text("authorization")
        .and_then(|raw| raw.split_once(' ').map(|(_, token)| token.trim().to_string()))
        .ok_or_else(missing)?;
    // The service verifies the token against the node's own locator (the same check the node's
    // package fetch gets) and answers the lock only after it matches, so a wrong token learns
    // nothing about which lock a node applies.
    let lock = node_connect_service::egress_lock_for_node(&state.services, node_id, &token).await?;
    Ok(Json(serde_json::json!({ "lock": lock })))
}
