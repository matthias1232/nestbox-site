//! Provider accounts, rented machines, cloud-init (ADR 0050).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::provision_service::{self, AccountView, HostView};
use crate::state::AppState;

pub async fn accounts(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = provision_service::accounts(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items, "kinds": crate::hosting::KINDS })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AccountRequest {
    /// `hetzner` or `proxmox`.
    pub kind: String,
    pub name: String,
    /// Non-secret settings: for Proxmox `api_url`, `node`, `template_vmid`, `address_pool`, `gateway`.
    #[serde(default)]
    pub config: serde_json::Value,
    /// The API token. Stored encrypted, never returned.
    pub token: String,
    /// Thousandths of a currency unit per hour for all running machines together; 0 is no cap.
    #[serde(default)]
    pub max_hourly_milli: i64,
}

pub async fn add_account(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<AccountRequest>,
) -> Result<(StatusCode, Json<AccountView>), ApiError> {
    let view = provision_service::add_account(&state.services, &auth.principal, &body.kind, &body.name, body.config, &body.token, body.max_hourly_milli).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

pub async fn remove_account(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    provision_service::remove_account(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn types(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<Json<serde_json::Value>, ApiError> {
    let items = provision_service::types(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn hosts(State(state): State<AppState>, auth: Auth) -> Result<Json<serde_json::Value>, ApiError> {
    let items = provision_service::hosts(&state.services, &auth.principal, None).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BurstRequest {
    pub account_id: Uuid,
    pub node_name: String,
    pub site_id: Uuid,
    pub host_type: String,
    #[serde(default)]
    pub location: Option<String>,
    /// How long the machine may live, in minutes (15 to 10080).
    #[serde(default = "default_ttl")]
    pub ttl_minutes: i64,
    /// Delete it after this many minutes with nothing on it; 0 turns that off.
    #[serde(default = "default_idle")]
    pub idle_minutes: i32,
}

fn default_ttl() -> i64 {
    240
}
fn default_idle() -> i32 {
    60
}

pub async fn burst(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<BurstRequest>,
) -> Result<(StatusCode, Json<HostView>), ApiError> {
    let host = provision_service::burst(
        &state.services,
        &state.config,
        &auth.principal,
        body.account_id,
        &body.node_name,
        body.site_id,
        &body.host_type,
        body.location.as_deref(),
        body.ttl_minutes,
        body.idle_minutes,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(host)))
}

pub async fn destroy(State(state): State<AppState>, auth: Auth, Path(id): Path<Uuid>) -> Result<StatusCode, ApiError> {
    provision_service::destroy(&state.services, &state.config, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The cloud-init document for a join token. Public: the token is the credential.
pub async fn cloud_init(State(state): State<AppState>, Path(token): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let text = provision_service::cloud_init_for_token(&state.services, &state.config, &token).await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/cloud-config; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((StatusCode::OK, headers, text))
}
