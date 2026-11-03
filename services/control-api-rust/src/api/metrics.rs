//! Resource history and the process table of one sandbox.
//!
//! Two shapes, two sources: the history is stored (raw for an hour, rolled up for
//! a week) and the process table is live from the runner. Both are bounded — the
//! history by its point budget, the table by the runner's own cap — so neither
//! can answer with a document the console cannot draw.
//!
//! The ingest route is not session-authenticated: a node presents its own token,
//! so it carries no CSRF requirement and accepts samples only for the sandboxes
//! in its environment.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::metrics::{MetricBatch, MetricRange, MetricSeries, ProcessTable};
use crate::error::ApiError;
use crate::services::metrics_service::{self, IngestOutcome};
use crate::state::AppState;

/// Query for one sandbox's metrics.
#[derive(Debug, Deserialize)]
pub struct MetricsQuery {
    /// `1h`, `24h`, or `7d`. Defaults to `1h`.
    pub range: Option<String>,
    /// A step override in seconds. The server clamps it to the window.
    pub step: Option<u32>,
}

/// CPU, memory, network and disk over a window.
pub async fn metrics(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<MetricsQuery>,
) -> Result<Json<MetricSeries>, ApiError> {
    // An unknown window is a 422 rather than a silent fallback to an hour: a
    // console that asks for something else than it draws is a bug worth naming.
    let range = match query.range.as_deref() {
        None => MetricRange::OneHour,
        Some(raw) => MetricRange::parse(raw).ok_or_else(|| {
            ApiError::validation("The range must be one of 1h, 24h, or 7d.")
        })?,
    };

    let series = metrics_service::series(&state.services, &auth.principal, id, range, query.step)
        .await?;
    Ok(Json(series))
}

/// Record a batch of samples from a node.
///
/// Accepted rather than created: the batch is a report, not a resource. The
/// answer says what was stored and what was dropped, because a node that keeps
/// sending unknown sandbox names is a deployment problem the operator can only
/// see if the count comes back.
pub async fn ingest(
    State(state): State<AppState>,
    auth: Auth,
    Json(batch): Json<MetricBatch>,
) -> Result<(StatusCode, Json<IngestOutcome>), ApiError> {
    let outcome = metrics_service::ingest(&state.services, &auth.principal, batch).await?;
    Ok((StatusCode::ACCEPTED, Json(outcome)))
}

/// The sandbox's process table, read live through the runner.
pub async fn processes(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<ProcessTable>, ApiError> {
    let table = metrics_service::processes(&state.services, &auth.principal, id).await?;
    Ok(Json(table))
}

/// A live, read-only view of one sandbox: its sockets and the processes that own them.
pub async fn live_view(
    State(state): State<AppState>,
    auth: Auth,
    Path((id, kind)): Path<(Uuid, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(metrics_service::live_view(&state.services, &auth.principal, id, &kind).await?))
}

#[derive(Debug, Deserialize)]
pub struct ProcessActionRequest {
    pub action: String,
    pub nice: Option<i8>,
}

/// Controlled process management. The fixed verbs are forwarded to the runner;
/// no user input is ever treated as a command.
pub async fn process_action(
    State(state): State<AppState>, auth: Auth, Path((id, pid)): Path<(Uuid, i64)>,
    Json(body): Json<ProcessActionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = metrics_service::process_action(&state.services, &auth.principal, id, pid, body.action, body.nice).await?;
    Ok(Json(result))
}


/// Record a batch of samples from a node, authenticated by the node's token.
///
/// The runner is a node, not a user: it presents the same node id and token as
/// the proxy's policy check, and names the environment it reports for. The batch
/// is then ingested exactly as a user token's would be, by a principal that exists
/// only for this call.
pub async fn ingest_node(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(batch): Json<MetricBatch>,
) -> Result<(StatusCode, Json<IngestOutcome>), ApiError> {
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
    let environment_id = text("x-nestbox-environment")
        .and_then(|value| Uuid::parse_str(&value).ok())
        .ok_or_else(missing)?;
    let token = text("authorization")
        .and_then(|raw| raw.split_once(' ').map(|(_, token)| token.trim().to_string()))
        .ok_or_else(missing)?;
    crate::services::policy_service::authenticate_node(&state.services, node_id, &token).await?;

    let principal = crate::domain::auth::Principal {
        user_id: Uuid::nil(),
        environment_id,
        role: crate::domain::auth::Role::Operator,
        display_name: format!("node {node_id}"),
        via: crate::domain::auth::CredentialSource::Token,
    };
    let outcome = metrics_service::ingest(&state.services, &principal, batch).await?;
    Ok((StatusCode::ACCEPTED, Json(outcome)))
}
