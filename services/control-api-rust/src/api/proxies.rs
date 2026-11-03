//! Exit proxies over HTTP.
//!
//! Two resources share one file because they are the same question from two
//! sides: the catalogue (`/proxies`), and which entry a sandbox or a provider
//! uses. A handler here decides nothing; it parses and delegates.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::proxy::{NewProxy, Proxy, ProxyKind, SandboxProxy};
use crate::error::ApiError;
use crate::services::proxy_service;
use crate::state::AppState;

/// Every exit in the catalogue.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = proxy_service::list(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// One exit.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Proxy>, ApiError> {
    Ok(Json(
        proxy_service::get(&state.services, &auth.principal, id).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProxyRequest {
    pub name: String,
    /// `http` or `socks5`.
    pub kind: String,
    pub host: String,
    pub port: i32,
    pub country: Option<String>,
    pub provider: Option<String>,
    pub username: Option<String>,
    /// A locator such as `file:///etc/nestbox/secrets/exit-de`. Never a value.
    pub password_ref: Option<String>,
}

impl ProxyRequest {
    fn into_new(self) -> Result<NewProxy, ApiError> {
        let kind = ProxyKind::parse(&self.kind).ok_or_else(|| {
            ApiError::validation("An exit is 'http' or 'socks5'.")
        })?;
        Ok(NewProxy {
            name: self.name,
            kind,
            host: self.host,
            port: self.port,
            country: self.country,
            provider: self.provider,
            username: self.username,
            password_ref: self.password_ref,
        })
    }
}

/// Add an exit to the catalogue.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<ProxyRequest>,
) -> Result<(StatusCode, Json<Proxy>), ApiError> {
    let proxy = proxy_service::create(&state.services, &auth.principal, body.into_new()?).await?;
    Ok((StatusCode::CREATED, Json(proxy)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProxyRequest {
    pub name: String,
    pub kind: String,
    pub host: String,
    pub port: i32,
    pub country: Option<String>,
    pub provider: Option<String>,
    pub username: Option<String>,
    pub password_ref: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub version: i64,
}

fn default_enabled() -> bool {
    true
}

/// Change an exit.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateProxyRequest>,
) -> Result<Json<Proxy>, ApiError> {
    let enabled = body.enabled;
    let version = body.version;
    let new = ProxyRequest {
        name: body.name,
        kind: body.kind,
        host: body.host,
        port: body.port,
        country: body.country,
        provider: body.provider,
        username: body.username,
        password_ref: body.password_ref,
    }
    .into_new()?;
    Ok(Json(
        proxy_service::update(&state.services, &auth.principal, id, version, new, enabled).await?,
    ))
}

/// Remove an exit.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    proxy_service::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The exit a sandbox uses.
pub async fn get_sandbox_proxy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<SandboxProxy>, ApiError> {
    Ok(Json(
        proxy_service::for_sandbox(&state.services, &auth.principal, id).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SandboxProxyRequest {
    /// `null` puts the sandbox back on the node's own route.
    pub proxy_id: Option<Uuid>,
}

/// Choose the exit a sandbox leaves through.
pub async fn put_sandbox_proxy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<SandboxProxyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        proxy_service::set_for_sandbox(&state.services, &auth.principal, id, body.proxy_id).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProviderProxyRequest {
    /// `null` calls the provider from the node's own route.
    pub proxy_id: Option<Uuid>,
}

/// Choose the exit the control plane uses when it calls a provider.
pub async fn put_provider_proxy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<ProviderProxyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        proxy_service::set_for_provider(&state.services, &auth.principal, id, body.proxy_id).await?,
    ))
}
