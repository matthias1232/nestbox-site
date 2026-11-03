//! Content privacy (DLP): the policy, a single filter test, incidents, purge.
//!
//! The handlers parse and delegate; the rules live in `services::privacy_service`
//! and the pure mode matrix in `domain::privacy`. Nothing here ever returns a
//! detected value: the service hands back masked strings and fingerprints only.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::privacy::{Detector, PrivacySettings, PrivacyUpdate};
use crate::error::ApiError;
use crate::services::privacy_service::{self, InspectResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ScopeQuery {
    pub sandbox_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One detector as the console lists it.
#[derive(Debug, Serialize, ToSchema)]
pub struct DetectorView {
    pub key: String,
    pub label: String,
    pub default_action: String,
}

/// The settings screen: the effective row and the catalog it refers to.
#[derive(Debug, Serialize, ToSchema)]
pub struct SettingsView {
    pub settings: PrivacySettings,
    pub detectors: Vec<DetectorView>,
}

fn catalog() -> Vec<DetectorView> {
    Detector::ALL
        .iter()
        .map(|detector| DetectorView {
            key: detector.as_str().to_string(),
            label: detector.label().to_string(),
            default_action: detector.default_action().as_str().to_string(),
        })
        .collect()
}

/// The environment default, or one sandbox's effective settings.
pub async fn get_settings(
    State(state): State<AppState>,
    auth: Auth,
    Query(scope): Query<ScopeQuery>,
) -> Result<Json<SettingsView>, ApiError> {
    let settings =
        privacy_service::effective_settings(&state.services, &auth.principal, scope.sandbox_id).await?;
    Ok(Json(SettingsView { settings, detectors: catalog() }))
}

/// A save request: the scope from the body, the values from the update.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveBody {
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    #[serde(flatten)]
    pub update: PrivacyUpdate,
}

/// Replace the policy for the environment (`sandbox_id` absent) or one sandbox.
pub async fn put_settings(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<SaveBody>,
) -> Result<Json<SettingsView>, ApiError> {
    let settings = privacy_service::set_settings(
        &state.services,
        &auth.principal,
        body.sandbox_id,
        body.update,
    )
    .await?;
    Ok(Json(SettingsView { settings, detectors: catalog() }))
}

/// Drop a sandbox's override so it inherits the environment default again.
pub async fn clear_settings(
    State(state): State<AppState>,
    auth: Auth,
    Path(sandbox_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    privacy_service::clear_settings(&state.services, &auth.principal, sandbox_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One filter test.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InspectBody {
    pub sandbox_id: Uuid,
    /// `inbound` (into the model) or `outbound` (back to the agent).
    pub direction: String,
    pub content: String,
    #[serde(default)]
    pub destination: Option<String>,
}

/// Run the policy over one piece of text and answer what would be sent.
pub async fn inspect(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<InspectBody>,
) -> Result<Json<InspectResult>, ApiError> {
    let direction = privacy_service::Direction::parse(&body.direction)
        .ok_or_else(|| ApiError::validation("direction must be 'inbound' or 'outbound'."))?;
    // The test button shows what the policy would do; the values behind the
    // placeholders never leave this endpoint.
    let mut result = privacy_service::inspect(
        &state.services,
        &auth.principal,
        body.sandbox_id,
        direction,
        &body.content,
        body.destination.as_deref(),
    )
    .await?;
    result.restore.clear();
    Ok(Json(result))
}

/// The recent incidents of one sandbox, newest first.
pub async fn get_incidents(
    State(state): State<AppState>,
    auth: Auth,
    Query(scope): Query<ScopeQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let sandbox_id = scope
        .sandbox_id
        .ok_or_else(|| ApiError::validation("sandbox_id is required."))?;
    let items = privacy_service::list_incidents(
        &state.services,
        &auth.principal,
        sandbox_id,
        scope.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// How many days of incidents to keep.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PurgeBody {
    #[serde(default = "default_days")]
    pub days: i64,
}

fn default_days() -> i64 {
    30
}

/// Delete incidents older than `days`. Admin only.
pub async fn purge(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<PurgeBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let removed = privacy_service::purge(&state.services, &auth.principal, body.days).await?;
    Ok(Json(serde_json::json!({ "removed": removed, "older_than_days": body.days })))
}


/// The DLP dashboard's query.
#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    #[serde(default)]
    pub timeframe: Option<String>,
    /// detector, action, severity, sandbox, day or destination. Default: detector.
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
}

/// Incidents over a window, grouped: the numbers a dashboard draws.
pub async fn dlp_overview(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<privacy_service::DlpReport>, ApiError> {
    let timeframe = crate::api::accounting::parse_timeframe(query.timeframe.as_deref())?;
    let since = timeframe.since(chrono::Utc::now());
    Ok(Json(
        privacy_service::dlp_overview(
            &state.services,
            &auth.principal,
            since,
            query.group_by.as_deref().unwrap_or("detector"),
            query.sandbox_id,
        )
        .await?,
    ))
}

/// The DLP log's query.
#[derive(Debug, Deserialize)]
pub struct LogQuery {
    #[serde(default)]
    pub timeframe: Option<String>,
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    /// blocked, redacted, replaced or allowed.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub detector: Option<String>,
    /// inbound (to the model) or outbound (to the agent).
    #[serde(default)]
    pub direction: Option<String>,
    /// egress, accounting_proxy, mcp or test.
    #[serde(default)]
    pub source: Option<String>,
    /// A free-text needle: detector, sandbox, destination, model, served model
    /// or owner name. Case-insensitive.
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// The newest incidents with their masked excerpts, across sandboxes.
pub async fn dlp_log(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<LogQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let timeframe = crate::api::accounting::parse_timeframe(query.timeframe.as_deref())?;
    let since = timeframe.since(chrono::Utc::now());
    let items = privacy_service::dlp_log(
        &state.services,
        &auth.principal,
        since,
        query.sandbox_id,
        query.action.as_deref().filter(|value| !value.is_empty() && *value != "all"),
        query.detector.as_deref().filter(|value| !value.is_empty() && *value != "all"),
        query.direction.as_deref().filter(|value| !value.is_empty() && *value != "all"),
        query.source.as_deref().filter(|value| !value.is_empty() && *value != "all"),
        query
            .q
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        query.limit.unwrap_or(100),
    )
    .await?;
    Ok(Json(serde_json::json!({ "items": items })))
}

/// The policy history's query.
#[derive(Debug, Deserialize)]
pub struct ChangesQuery {
    #[serde(default)]
    pub sandbox_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Who changed this sandbox's DLP policy (or the default it inherits), when, and
/// from what to what.
///
/// The audit trail is the source: a save, an inherit and a purge are audited
/// acts already, and a second table of the same facts would be a second truth.
pub async fn dlp_changes(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<ChangesQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let items = privacy_service::changes(
        &state.services,
        &auth.principal,
        query.sandbox_id,
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(serde_json::json!({ "items": items })))
}
