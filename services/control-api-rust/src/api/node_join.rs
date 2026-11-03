//! Joining nodes (ADR 0047): ask for a token, run the script, the machine appears.
//!
//! The two `join` routes are public on purpose: the machine has no account. What protects them
//! is the token, which is long, single-use, short-lived, and bound to one data center and site.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::node_join_service::{self, JoinAnswer, JoinToken};
use crate::state::AppState;

#[derive(Debug, Deserialize, ToSchema)]
pub struct JoinTokenRequest {
    /// Lowercase letters, digits and dashes.
    pub node_name: String,
    pub site_id: Uuid,
}

/// Ask for a one-time link that joins one machine to a site of this data center.
pub async fn create_token(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<JoinTokenRequest>,
) -> Result<(StatusCode, Json<JoinToken>), ApiError> {
    let made = node_join_service::create_token(&state.services, &state.config, &auth.principal, &body.node_name, body.site_id).await?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// The install script for a token. Public; the token is the credential.
pub async fn script(State(state): State<AppState>, Path(token): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let text = node_join_service::script(&state.services, &state.config, &token).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/x-shellscript; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, headers, text))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct JoinRequest {
    /// The node's WireGuard public key. The private key never leaves the node.
    pub public_key: String,
    #[serde(default)]
    pub hostname: String,
}

/// Trade the token for an identity. Public; single use.
pub async fn join(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<JoinRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let answer: JoinAnswer = node_join_service::join(&state.services, &state.config, &token, &body.public_key, &body.hostname).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, headers, Json(answer)))
}

/// Remove a joined node that has nothing running on it.
pub async fn remove(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    node_join_service::remove(&state.services, &state.config, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The node package as it should be now. The node proves itself with its token.
pub async fn compose(State(state): State<AppState>, headers: HeaderMap) -> Result<impl IntoResponse, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .trim()
        .to_string();
    let node = headers
        .get("x-nestbox-node")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v.trim()).ok())
        .ok_or_else(|| ApiError::unauthenticated("The node token was refused."))?;
    let text = node_join_service::compose_for(&state.services, node, &token).await?;
    let mut out = HeaderMap::new();
    out.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/yaml; charset=utf-8"));
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, out, text))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveRequest {
    /// The name of the node to move the sandbox to.
    pub node: String,
}

/// Move a sandbox and its persistent folders to another node. Returns at once.
pub async fn move_sandbox(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<MoveRequest>,
) -> Result<(StatusCode, Json<crate::services::move_service::MoveView>), ApiError> {
    let started = crate::services::move_service::start(&state.services, &state.config, &auth.principal, id, &body.node).await?;
    Ok((StatusCode::ACCEPTED, Json(started)))
}

/// The recent moves of this data center.
pub async fn moves(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = crate::services::move_service::list(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// One move with its step log.
pub async fn get_move(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<Json<crate::services::move_service::MoveView>, ApiError> {
    Ok(Json(crate::services::move_service::get(&state.services, &auth.principal, id).await?))
}

/// The snapshots of one persistent folder.
pub async fn root_snapshots(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<Json<serde_json::Value>, ApiError> {
    let items = crate::services::snapshot_service::list(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PruneRequest {
    #[serde(default = "keep_last")]
    pub keep_last: i32,
    #[serde(default = "keep_daily")]
    pub keep_daily: i32,
}
fn keep_last() -> i32 {
    20
}
fn keep_daily() -> i32 {
    14
}

/// Forget old snapshots of this data center and reclaim the space.
pub async fn prune_snapshots(State(state): State<AppState>, auth: Auth, Json(body): Json<PruneRequest>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(crate::services::snapshot_service::prune(&state.services, &auth.principal, body.keep_last, body.keep_daily).await?))
}

/// Bring a sandbox back on another node after its node is gone, from the newest scheduled snapshots.
pub async fn recover_sandbox(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<MoveRequest>,
) -> Result<(StatusCode, Json<crate::services::move_service::MoveView>), ApiError> {
    let started = crate::services::move_service::recover(&state.services, &state.config, &auth.principal, id, &body.node).await?;
    Ok((StatusCode::ACCEPTED, Json(started)))
}
