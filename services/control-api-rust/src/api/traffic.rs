//! Traffic, the live map, effective policies, and the MCP calls the proxy saw.
//!
//! These are the read paths the Admin Center draws from. Each one takes a
//! window, returns a bounded page, and is aggregated on the server where the
//! console would otherwise count rows in the browser. The handlers parse and
//! delegate; the window, the page cap, the tenant scope, and the deployed
//! comparison live in `services::traffic_service`.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::error::ApiError;
use crate::services::traffic_service;
use crate::services::traffic_service::{
    BodyView, EffectivePolicies, MapView, McpCallsPage, TrafficPage,
};
use crate::state::AppState;

/// A window plus the filters the Traffic tab offers.
#[derive(Debug, Deserialize)]
pub struct TrafficQuery {
    /// `5m`, `15m`, `1h`, `24h`. Defaults to `5m`, capped at `24h`.
    pub window: Option<String>,
    /// `all`, `blocked`, `allowed`, `mcp`. Defaults to `all`.
    pub filter: Option<String>,
    /// A fragment of the destination host, case-insensitive.
    pub host: Option<String>,
    /// An exact MCP tool name, for the MCP view.
    pub tool: Option<String>,
    /// Opaque cursor from the previous page.
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl TrafficQuery {
    /// The fields the traffic list needs.
    fn traffic_request(&self) -> traffic_service::TrafficRequest {
        traffic_service::TrafficRequest {
            window: self.window.clone(),
            filter: self.filter.clone(),
            host: self.host.clone(),
            cursor: self.cursor.clone(),
            limit: self.limit,
        }
    }

    /// The fields the MCP call list needs.
    fn mcp_request(&self) -> traffic_service::McpCallsRequest {
        traffic_service::McpCallsRequest {
            window: self.window.clone(),
            host: self.host.clone(),
            tool: self.tool.clone(),
            cursor: self.cursor.clone(),
            limit: self.limit,
        }
    }
}

/// The environment-wide live list: every sandbox at once.
///
/// One endpoint rather than a loop over the sandboxes, because "what is
/// happening now" is a question about the environment and a browser-side merge
/// would be both slower and a second definition of the order.
#[derive(Debug, Deserialize)]
pub struct EnvironmentTrafficQuery {
    /// `5m`, `15m`, `1h`, `24h`. Defaults to `5m`, capped at `24h`.
    pub window: Option<String>,
    /// `all`, `blocked`, `allowed`, `mcp`. Defaults to `all`.
    pub filter: Option<String>,
    /// A fragment of the destination host, case-insensitive.
    pub host: Option<String>,
    /// Restrict to one sandbox, still tenant-scoped.
    pub sandbox: Option<Uuid>,
    /// Opaque cursor from the previous page.
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

impl EnvironmentTrafficQuery {
    fn request(&self) -> traffic_service::EnvironmentTrafficRequest {
        traffic_service::EnvironmentTrafficRequest {
            window: self.window.clone(),
            filter: self.filter.clone(),
            host: self.host.clone(),
            sandbox_id: self.sandbox,
            cursor: self.cursor.clone(),
            limit: self.limit,
        }
    }
}

/// What the decisions view asks for: a window, and optionally one sandbox.
#[derive(Debug, Deserialize)]
pub struct DecisionsQuery {
    /// `5m`, `15m`, `1h`, `24h`. Defaults to `1h`, capped at `24h`.
    pub window: Option<String>,
    /// Restrict to one sandbox, still tenant-scoped.
    pub sandbox: Option<Uuid>,
}

/// The destinations that were denied, grouped, for the decisions view.
pub async fn decisions(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<DecisionsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let answer = traffic_service::decisions(
        &state.services,
        &auth.principal,
        traffic_service::DecisionsRequest {
            window: query.window.clone(),
            sandbox_id: query.sandbox,
        },
    )
    .await?;
    Ok(Json(answer))
}

/// Traffic across every sandbox of the caller's environment.
pub async fn environment(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<EnvironmentTrafficQuery>,
) -> Result<Json<traffic_service::EnvironmentTrafficPage>, ApiError> {
    let page =
        traffic_service::environment_traffic(&state.services, &auth.principal, query.request())
            .await?;
    Ok(Json(page))
}

/// Egress traffic rows for one sandbox, with the window's counts.
pub async fn traffic(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<TrafficQuery>,
) -> Result<Json<TrafficPage>, ApiError> {
    let page = traffic_service::traffic(
        &state.services,
        &auth.principal,
        id,
        query.traffic_request(),
    )
    .await?;
    Ok(Json(page))
}

/// The redacted request and response body behind one traffic row.
pub async fn body(
    State(state): State<AppState>,
    auth: Auth,
    Path((sandbox, event)): Path<(Uuid, Uuid)>,
) -> Result<Json<BodyView>, ApiError> {
    let body = traffic_service::body(&state.services, &auth.principal, sandbox, event).await?;
    Ok(Json(body))
}

/// Aggregated egress edges for the live map, with their totals.
pub async fn map(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<TrafficQuery>,
) -> Result<Json<MapView>, ApiError> {
    let view = traffic_service::map(
        &state.services,
        &auth.principal,
        id,
        query.window.as_deref(),
        query.limit,
    )
    .await?;
    Ok(Json(view))
}

/// The rules that apply to this sandbox, with the proxy's deployment state.
pub async fn effective_policies(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<EffectivePolicies>, ApiError> {
    let policies =
        traffic_service::effective_policies(&state.services, &auth.principal, id).await?;
    Ok(Json(policies))
}

/// MCP calls the proxy saw for this sandbox, with redacted arguments.
pub async fn mcp_calls(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<TrafficQuery>,
) -> Result<Json<McpCallsPage>, ApiError> {
    let page = traffic_service::mcp_calls(
        &state.services,
        &auth.principal,
        id,
        query.mcp_request(),
    )
    .await?;
    Ok(Json(page))
}
