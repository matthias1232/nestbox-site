//! Live traffic, the map, the MCP calls, and the rules in force.
//!
//! This module answers the Admin Center's read questions about egress. Four
//! rules shape every function here, and each exists because the alternative is a
//! screen that lies:
//!
//! 1. **A window is required, and capped.** `?window=5m` is the normal case; a
//!    request without one gets five minutes, not "all of history". Twenty-four
//!    hours is the ceiling for rows and one hour for the map, because the map is
//!    a picture of *now* and a trend belongs in a chart with a range parameter.
//! 2. **Aggregates are computed by the database.** The console draws counts, not
//!    rows; a browser counting a five-minute window at ten thousand events a
//!    minute is a browser that stops responding.
//! 3. **A foreign sandbox is a 404.** Every store query already filters by
//!    environment and sandbox, so a sandbox of another tenant would return an
//!    empty page. Empty and not-yours must not look the same.
//! 4. **The deployed badge is a comparison, not an opinion.** The node reports
//!    the hash of the rule set it enforces; the control API records the hash it
//!    compiled. Equal means deployed, anything else — including "no report" —
//!    means not.
//!
//! Bodies and MCP arguments pass the central redactor again on the way out. They
//! were redacted before the insert; doing it twice is free next to the cost of
//! being wrong once.

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use super::geography_service;
use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::egress::{Decision, OpenWindow, TrafficEdge, TrafficEvent};
use crate::domain::mcp::McpToolCall;
use crate::domain::pagination::{Cursor, CursorKey};
use crate::error::ApiError;
use crate::security::redaction;
use crate::store::postgres::traffic as store;
use crate::store::TenantScope;

/// The window used when the request names none: the tab is a live view.
pub const DEFAULT_WINDOW: &str = "5m";

/// The longest window the row list will read. A day of one sandbox's traffic is
/// already a lot of rows; a week of it is an archive, not a list.
pub const MAX_WINDOW_SECONDS: i64 = 24 * 60 * 60;

/// The longest window the map and the live graph will read. They answer "what is
/// happening", and an hour of edges is generous for that.
pub const MAX_LIVE_WINDOW_SECONDS: i64 = 60 * 60;

/// The window a read covers, expressed as a length.
///
/// Relative rather than absolute so one request has one clock reading: the
/// `from` and `to` the response reports are the ones the query used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub seconds: i64,
}

impl Window {
    /// The default window, five minutes.
    pub fn default_window() -> Self {
        Self { seconds: 5 * 60 }
    }

    /// Cap the window at a maximum, for the views that are about "now".
    pub fn capped(self, max_seconds: i64) -> Self {
        Self {
            seconds: self.seconds.min(max_seconds),
        }
    }

    /// The window as `{"from", "to"}` against one clock reading.
    pub fn bounds(self) -> (DateTime<Utc>, DateTime<Utc>) {
        let to = Utc::now();
        (to - Duration::seconds(self.seconds), to)
    }

    /// The canonical label: `30s`, `5m`, `1h`.
    ///
    /// Normalised on the way out so the console compares two responses by
    /// string and a capped window is visibly capped.
    pub fn label(self) -> String {
        if self.seconds % 3600 == 0 {
            format!("{}h", self.seconds / 3600)
        } else if self.seconds % 60 == 0 {
            format!("{}m", self.seconds / 60)
        } else {
            format!("{}s", self.seconds)
        }
    }
}

/// Parse a window parameter such as `5m`, `90s`, or `2h`.
///
/// A missing value is the default; a value that is not a positive duration is a
/// validation error rather than a silent fallback, because a client that asked
/// for `5min` and got five minutes of something else would not notice.
pub fn parse_window(value: Option<&str>) -> Result<Window, ApiError> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Window::default_window());
    };

    let split_at = raw
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| ApiError::validation("A window must end in s, m, or h, for example 5m."))?;
    let (digits, suffix) = raw.split_at(split_at);
    let amount: i64 = digits.parse().map_err(|_| {
        ApiError::validation("A window must start with a whole number, for example 5m.")
    })?;
    let unit = match suffix {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        _ => {
            return Err(ApiError::validation(
                "A window must end in s, m, or h, for example 5m.",
            ))
        }
    };
    let seconds = amount.saturating_mul(unit);
    if seconds < 1 {
        return Err(ApiError::validation("A window must be longer than zero."));
    }
    Ok(Window {
        seconds: seconds.min(MAX_WINDOW_SECONDS),
    })
}

/// Which rows the list keeps, parsed from the filter parameter.
fn parse_filter(value: Option<&str>) -> store::Filter {
    match value.map(str::trim) {
        Some("blocked") => store::Filter::Blocked,
        Some("allowed") => store::Filter::Allowed,
        Some("mcp") => store::Filter::Mcp,
        _ => store::Filter::All,
    }
}

/// A page size, clamped to what the store will return.
fn page_size(value: Option<u32>, default: i64) -> i64 {
    let requested = value.map(i64::from).unwrap_or(default);
    requested.clamp(1, store::MAX_LIMIT)
}

/// The cursor key of the previous page, if the client sent a usable one.
///
/// A malformed cursor is ignored rather than rejected: it is opaque client
/// state, and a 400 on a page request would strand an operator mid-scroll. The
/// response always carries a fresh cursor.
fn cursor_of(encoded: Option<&str>) -> Option<(DateTime<Utc>, Uuid)> {
    let cursor = Cursor::decode(encoded?)?;
    match cursor.key {
        CursorKey::Timestamp(ts) => Some((ts, cursor.id)),
        CursorKey::Text(_) => None,
    }
}

/// The cursor that fetches the page after this one.
///
/// Only when the page was full: a short page is the last page, and saying so
/// saves a round trip that would return nothing.
fn next_cursor(rows: &[TrafficEvent], limit: i64) -> Option<String> {
    if rows.len() as i64 != limit {
        return None;
    }
    let last = rows.last()?;
    Some(
        Cursor {
            key: CursorKey::Timestamp(last.ts),
            id: last.id,
        }
        .encode(),
    )
}

/// Refuse a read for a sandbox that is not this tenant's.
///
/// Every query is already scoped, so a foreign sandbox would answer with an
/// empty page. That is the wrong answer twice: it hides a typo, and it makes
/// "nothing happened yet" indistinguishable from "not yours".
async fn ensure_sandbox(scope: &mut TenantScope<'_>, sandbox_id: Uuid) -> Result<(), ApiError> {
    if store::sandbox_exists(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(());
    }
    Err(ApiError::not_found(
        crate::error::ErrorCode::SANDBOX_NOT_FOUND,
        "The sandbox does not exist in this environment.",
    ))
}

/// What one traffic page carries.
#[derive(Debug, serde::Serialize)]
pub struct TrafficPage {
    pub sandbox_id: Uuid,
    pub window: String,
    pub window_seconds: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub limit: i64,
    pub filter: String,
    pub host: Option<String>,
    pub items: Vec<TrafficEvent>,
    pub next_cursor: Option<String>,
    pub summary: store::Summary,
}

/// The filters one traffic read takes, as the handler parsed them.
pub struct TrafficRequest {
    pub window: Option<String>,
    pub filter: Option<String>,
    pub host: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Read one page of traffic rows plus the window's counts.
///
/// The counts cover the whole window and `summary.matching` says how many rows
/// the filter keeps, so the tab can show "7 blocked of 1 204" without a second
/// request and without counting in the browser.
pub async fn traffic(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    request: TrafficRequest,
) -> Result<TrafficPage, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(request.window.as_deref())?.capped(MAX_WINDOW_SECONDS);
    let filter = parse_filter(request.filter.as_deref());
    let limit = page_size(request.limit, 100);
    let host = request
        .host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    let after = cursor_of(request.cursor.as_deref());

    let (from, to) = window.bounds();
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    ensure_sandbox(&mut scope, sandbox_id).await?;

    let query = store::RowQuery {
        sandbox_id,
        since: from,
        until: to,
        filter,
        host,
        after,
        limit,
    };

    let mut items = store::rows(&mut scope, query)
        .await
        .map_err(ApiError::from)?;

    store::mark_bodies(&mut scope, sandbox_id, &mut items)
        .await
        .map_err(ApiError::from)?;

    let summary = store::summary(&mut scope, query)
        .await
        .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;

    // A country is filled in the background, never in this request: a registry
    // answers in hundreds of milliseconds on a good day, and the page has rows
    // to draw now. The next refresh shows what the registry said.
    let unresolved: Vec<String> = items
        .iter()
        .filter(|item| item.country.is_none())
        .map(|item| item.host.clone())
        .collect();
    geography_service::spawn_fill(ctx.pool.clone(), unresolved);

    Ok(TrafficPage {
        sandbox_id,
        window: window.label(),
        window_seconds: window.seconds,
        from,
        to,
        limit,
        filter: filter.as_str().to_string(),
        host: host.map(str::to_string),
        next_cursor: next_cursor(&items, limit),
        items,
        summary,
    })
}

/// One page of the environment-wide traffic list.
///
/// The same shape as `TrafficPage` minus the single sandbox id: this is the
/// view that answers "what is happening right now, everywhere", and every row
/// names its sandbox instead.
#[derive(Debug, serde::Serialize)]
pub struct EnvironmentTrafficPage {
    pub window: String,
    pub window_seconds: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub limit: i64,
    pub filter: String,
    pub host: Option<String>,
    pub sandbox_id: Option<Uuid>,
    pub items: Vec<TrafficEvent>,
    pub next_cursor: Option<String>,
    pub summary: store::Summary,
}

/// The filters one environment-wide traffic read takes.
pub struct EnvironmentTrafficRequest {
    pub window: Option<String>,
    pub filter: Option<String>,
    pub host: Option<String>,
    pub sandbox_id: Option<Uuid>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Read one page of traffic across every sandbox of the caller's environment.
///
/// This is the console's live list, and it is deliberately the same store and the
/// same window rules as the per-sandbox read: a second query shape would let the
/// two disagree about what counts as a request. The row cap is the environment
/// one because a single sandbox's day of traffic is already a lot of rows.
pub async fn environment_traffic(
    ctx: &ServiceContext,
    principal: &Principal,
    request: EnvironmentTrafficRequest,
) -> Result<EnvironmentTrafficPage, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(request.window.as_deref())?.capped(MAX_WINDOW_SECONDS);
    let filter = parse_filter(request.filter.as_deref());
    let limit = page_size(request.limit, 200);
    let host = request
        .host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    let after = cursor_of(request.cursor.as_deref());

    let (from, to) = window.bounds();
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    // A named sandbox that is not this tenant's is a 404, not an empty page: the
    // two must not look the same, here as everywhere else.
    if let Some(sandbox_id) = request.sandbox_id {
        ensure_sandbox(&mut scope, sandbox_id).await?;
    }

    let query = store::EnvironmentRowQuery {
        since: from,
        until: to,
        filter,
        host,
        sandbox_id: request.sandbox_id,
        after,
        limit,
    };

    let items = store::rows_environment(&mut scope, query)
        .await
        .map_err(ApiError::from)?;

    let summary = store::summary_environment(&mut scope, query)
        .await
        .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;

    // A country is filled in the background, never in this request, for the same
    // reason as the per-sandbox read: the page has rows to draw now.
    let unresolved: Vec<String> = items
        .iter()
        .filter(|item| item.country.is_none())
        .map(|item| item.host.clone())
        .collect();
    geography_service::spawn_fill(ctx.pool.clone(), unresolved);

    Ok(EnvironmentTrafficPage {
        window: window.label(),
        window_seconds: window.seconds,
        from,
        to,
        limit,
        filter: filter.as_str().to_string(),
        host: host.map(str::to_string),
        sandbox_id: request.sandbox_id,
        next_cursor: next_cursor(&items, limit),
        items,
        summary,
    })
}

/// What the decisions view asks for: a window, and optionally one sandbox.
pub struct DecisionsRequest {
    pub window: Option<String>,
    pub sandbox_id: Option<Uuid>,
}

/// The denied destinations in one window, worst first.
///
/// The decisions view is deliberately narrower than the traffic list: only what
/// was stopped, grouped by what a person can allow. Default window is one hour,
/// because a decision that is a day old is usually already resolved, and the cap
/// stays at a day for the same reason as the traffic list.
pub async fn decisions(
    ctx: &ServiceContext,
    principal: &Principal,
    request: DecisionsRequest,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(request.window.as_deref().or(Some("1h")))?
        .capped(MAX_WINDOW_SECONDS);
    let (from, to) = window.bounds();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if let Some(sandbox_id) = request.sandbox_id {
        ensure_sandbox(&mut scope, sandbox_id).await?;
    }
    let items = store::denied_targets(&mut scope, from, to, request.sandbox_id, store::MAX_EDGES)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let unresolved: Vec<String> = items
        .iter()
        .filter(|item| item.reason.is_none())
        .map(|item| item.host.clone())
        .collect();
    geography_service::spawn_fill(ctx.pool.clone(), unresolved);

    Ok(serde_json::json!({
        "window": window.label(),
        "window_seconds": window.seconds,
        "from": from,
        "to": to,
        "items": items,
    }))
}

/// One request and response body, after the redactor.
#[derive(Debug, serde::Serialize)]
pub struct BodyView {
    pub event_id: Uuid,
    pub captured_at: DateTime<Utc>,
    /// When the retention job will delete it: `captured_at` plus a day.
    pub retained_until: DateTime<Utc>,
    /// True when the proxy hit its buffer limit and this is a prefix.
    pub truncated: bool,
    pub request_bytes: i64,
    pub response_bytes: i64,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    /// Always true: the payload passed `security::redaction` before it left this
    /// process, whether or not the ingest path already did it.
    pub redacted: bool,
}

/// The body retention the migration and the maintenance job agree on.
const BODY_RETENTION_HOURS: i64 = 24;

/// Read the redacted bodies behind one traffic row.
///
/// A body that is not there is a 404, not an empty object: it may have expired,
/// or the proxy may not have captured one, and both are "no body" to a reader
/// who clicked "show body".
pub async fn body(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    event_id: Uuid,
) -> Result<BodyView, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    ensure_sandbox(&mut scope, sandbox_id).await?;

    let stored = store::body(&mut scope, sandbox_id, event_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let Some(stored) = stored else {
        return Err(ApiError::not_found(
            crate::error::ErrorCode::NOT_FOUND,
            "No body is stored for this event; it was not captured, or its 24-hour retention has passed.",
        ));
    };

    Ok(BodyView {
        event_id: stored.event_id,
        captured_at: stored.ts,
        retained_until: stored.ts + Duration::hours(BODY_RETENTION_HOURS),
        truncated: stored.truncated,
        request_bytes: stored.request_bytes,
        response_bytes: stored.response_bytes,
        request_body: stored.request_body.as_deref().map(redaction::redact),
        response_body: stored.response_body.as_deref().map(redaction::redact),
        redacted: true,
    })
}

/// What the live map shows: edges, and the totals over them.
#[derive(Debug, serde::Serialize)]
pub struct MapView {
    pub sandbox_id: Uuid,
    pub window: String,
    pub window_seconds: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub edges: Vec<TrafficEdge>,
    pub totals: MapTotals,
}

/// The numbers above the map, summed from the edges it returns.
#[derive(Debug, Default, serde::Serialize)]
pub struct MapTotals {
    pub requests: i64,
    pub allowed: i64,
    pub denied: i64,
    pub bytes: i64,
    pub destinations: i64,
    /// Destinations that were blocked at least once: what the red edges count.
    pub blocked_destinations: i64,
}

/// Aggregate one sandbox's last window of traffic into map edges.
pub async fn map(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    window: Option<&str>,
    limit: Option<u32>,
) -> Result<MapView, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(window)?.capped(MAX_LIVE_WINDOW_SECONDS);
    let (from, to) = window.bounds();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    ensure_sandbox(&mut scope, sandbox_id).await?;

    let edges = store::map_edges(
        &mut scope,
        sandbox_id,
        from,
        to,
        page_size(limit, 200).min(store::MAX_EDGES),
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut totals = MapTotals::default();
    for edge in &edges {
        totals.requests += edge.requests;
        totals.allowed += edge.allowed;
        totals.denied += edge.denied;
        totals.bytes += edge.bytes;
        totals.destinations += 1;
        if edge.denied > 0 {
            totals.blocked_destinations += 1;
        }
    }

    Ok(MapView {
        sandbox_id,
        window: window.label(),
        window_seconds: window.seconds,
        from,
        to,
        edges,
        totals,
    })
}

/// A page of MCP calls.
#[derive(Debug, serde::Serialize)]
pub struct McpCallsPage {
    pub sandbox_id: Uuid,
    pub window: String,
    pub window_seconds: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub limit: i64,
    pub items: Vec<McpToolCall>,
    pub next_cursor: Option<String>,
}

/// One MCP call list request, as the handler parsed it.
pub struct McpCallsRequest {
    pub window: Option<String>,
    pub host: Option<String>,
    pub tool: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// Read the MCP calls the proxy saw for one sandbox.
///
/// Arguments and results are redacted a second time on the way out. The toolbox
/// is what an agent does to the world, so this list is the most sensitive read
/// in the console; the redactor is the boundary and it runs on every path.
pub async fn mcp_calls(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    request: McpCallsRequest,
) -> Result<McpCallsPage, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(request.window.as_deref())?.capped(MAX_WINDOW_SECONDS);
    let limit = page_size(request.limit, 100);
    let server_host = request
        .host
        .as_deref()
        .map(str::trim)
        .filter(|host| !host.is_empty());
    let tool = request
        .tool
        .as_deref()
        .map(str::trim)
        .filter(|tool| !tool.is_empty());
    let after = cursor_of(request.cursor.as_deref());
    let (from, to) = window.bounds();

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    ensure_sandbox(&mut scope, sandbox_id).await?;

    let calls = store::mcp_calls(
        &mut scope,
        store::CallQuery {
            sandbox_id,
            since: from,
            until: to,
            server_host,
            tool,
            after,
            limit,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let items: Vec<McpToolCall> = calls
        .into_iter()
        .map(|mut call| {
            call.arguments = call.arguments.as_ref().map(redaction::redact_json);
            call.result = call.result.as_ref().map(redaction::redact_json);
            call
        })
        .collect();

    let next = if items.len() as i64 == limit {
        items.last().map(|last| {
            Cursor {
                key: CursorKey::Timestamp(last.ts),
                id: last.id,
            }
            .encode()
        })
    } else {
        None
    };

    Ok(McpCallsPage {
        sandbox_id,
        window: window.label(),
        window_seconds: window.seconds,
        from,
        to,
        limit,
        items,
        next_cursor: next,
    })
}

/// The deployed state of one node, as the Policies tab reports it.
#[derive(Debug, serde::Serialize)]
pub struct DeploymentView {
    /// Two hashes, compared. The only claim the badge makes.
    pub deployed: bool,
    /// One sentence the console shows verbatim, including the reason when the
    /// answer is "not deployed".
    pub explanation: String,
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
    pub reported_hash: Option<String>,
    pub reported_at: Option<DateTime<Utc>>,
    pub compiled_hash: Option<String>,
    pub compiled_at: Option<DateTime<Utc>>,
    pub rule_count: i64,
    pub window_count: i64,
}

/// One rule in force, with where it came from.
#[derive(Debug, serde::Serialize)]
pub struct EffectiveRuleView {
    pub id: Uuid,
    pub name: String,
    /// The policy the rule belongs to; `None` for a rule drafted for a sandbox
    /// that has not become part of a policy yet.
    pub policy: Option<String>,
    /// `sandbox`, `global`, or `environment` for a policy row that scopes to
    /// everything.
    pub scope: String,
    pub action: String,
    pub enforcement: String,
    pub host: String,
    pub port: Option<i32>,
    pub protocol: String,
    pub methods: Vec<String>,
    pub priority: i32,
    pub version: i64,
}

/// The rules that apply to one sandbox, the open windows, and the deployment.
#[derive(Debug, serde::Serialize)]
pub struct EffectivePolicies {
    pub sandbox_id: Uuid,
    pub deployment: DeploymentView,
    pub rules: Vec<EffectiveRuleView>,
    pub windows: Vec<OpenWindow>,
}

/// The first twelve characters of a hash, for a sentence.
fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

/// Build the deployed view, including the sentence that explains it.
fn deployment_view(deployment: Option<store::Deployment>) -> DeploymentView {
    let Some(deployment) = deployment else {
        return DeploymentView {
            deployed: false,
            explanation: "The sandbox does not exist in this environment.".to_string(),
            node_id: None,
            node_name: None,
            reported_hash: None,
            reported_at: None,
            compiled_hash: None,
            compiled_at: None,
            rule_count: 0,
            window_count: 0,
        };
    };

    let explanation = match (&deployment.reported_hash, &deployment.compiled_hash) {
        (Some(reported), Some(compiled)) if reported == compiled => format!(
            "The node enforces the compiled rule set ({}).",
            short_hash(reported)
        ),
        (Some(reported), Some(compiled)) => format!(
            "The node reports {} but the compiled set is {}: it is still enforcing an older rule set.",
            short_hash(reported),
            short_hash(compiled)
        ),
        (None, Some(compiled)) => format!(
            "The node has not reported a policy hash yet; the compiled set is {}.",
            short_hash(compiled)
        ),
        (_, None) => "No rule set has been compiled for this environment yet.".to_string(),
    };

    DeploymentView {
        deployed: deployment.is_deployed(),
        explanation,
        node_id: deployment.node_id,
        node_name: deployment.node_name,
        reported_hash: deployment.reported_hash,
        reported_at: deployment.reported_at,
        compiled_hash: deployment.compiled_hash,
        compiled_at: deployment.compiled_at,
        rule_count: deployment.rule_count,
        window_count: deployment.window_count,
    }
}

/// Read the rules in force for one sandbox.
///
/// "In force" is the decision function's definition: enabled rules whose
/// `sandbox_id` is this sandbox or null. A rule that `decide` would ignore is
/// not listed as effective, because the tab's job is to explain what the proxy
/// will do.
pub async fn effective_policies(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<EffectivePolicies, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    ensure_sandbox(&mut scope, sandbox_id).await?;

    let deployment = store::deployment(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let rules = store::effective_rules(&mut scope, sandbox_id, store::MAX_LIMIT)
        .await
        .map_err(ApiError::from)?;
    let windows = store::open_windows(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let rules = rules
        .into_iter()
        .map(|(rule, policy_name, policy_scope)| {
            let scope = match (rule.sandbox_id, policy_scope.as_deref()) {
                (Some(_), _) => "sandbox",
                (None, Some("global")) => "global",
                _ => "environment",
            };
            EffectiveRuleView {
                id: rule.id,
                name: rule.name,
                policy: policy_name,
                scope: scope.to_string(),
                action: rule.action.as_str().to_string(),
                enforcement: rule.enforcement.as_str().to_string(),
                host: rule.host,
                port: rule.port,
                protocol: rule.protocol.as_str().to_string(),
                methods: rule.methods,
                priority: rule.priority,
                version: rule.version,
            }
        })
        .collect();

    Ok(EffectivePolicies {
        sandbox_id,
        deployment: deployment_view(deployment),
        rules,
        windows,
    })
}

/// The environment-wide live edges the cluster graph adds.
#[derive(Debug, serde::Serialize)]
pub struct LiveEdges {
    pub window: String,
    pub window_seconds: i64,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub edges: Vec<TrafficEdge>,
    /// The sandbox name and node for each edge's sandbox, so the graph can label
    /// a node it has not seen in its static half.
    pub sandboxes: Vec<LiveSandbox>,
    /// True when the limit cut the list, so the console says "top connections"
    /// instead of implying it shows all of them.
    pub truncated: bool,
}

/// One sandbox that appears in the live edges.
#[derive(Debug, serde::Serialize)]
pub struct LiveSandbox {
    pub id: Uuid,
    pub name: String,
    pub node_id: Option<Uuid>,
}

/// Read the live edges of a whole environment, for `GET /topology?live=5m`.
///
/// Separate from `map` because the subject is different: this is "who talks to
/// what in this environment", capped and ordered by volume, and it must not fail
/// when one sandbox is quiet.
pub async fn live_edges(
    ctx: &ServiceContext,
    principal: &Principal,
    window: Option<&str>,
) -> Result<LiveEdges, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let window = parse_window(window)?.capped(MAX_LIVE_WINDOW_SECONDS);
    let (from, to) = window.bounds();
    let limit = 300i64;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = store::live_edges(&mut scope, from, to, limit + 1)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let truncated = rows.len() as i64 > limit;
    let mut sandboxes: Vec<LiveSandbox> = Vec::new();
    let mut edges: Vec<TrafficEdge> = Vec::new();
    for row in rows.into_iter().take(limit as usize) {
        if !sandboxes.iter().any(|known| known.id == row.sandbox_id) {
            sandboxes.push(LiveSandbox {
                id: row.sandbox_id,
                name: row.sandbox_name,
                node_id: row.node_id,
            });
        }
        edges.push(row.edge);
    }

    Ok(LiveEdges {
        window: window.label(),
        window_seconds: window.seconds,
        from,
        to,
        edges,
        sandboxes,
        truncated,
    })
}

/// Whether a decision stopped the connection, for the graph's colour.
///
/// A single place so the map and the console cannot disagree about what "red"
/// means: `deny` only. Audit-only rules pass, and colouring them as blocks would
/// teach an operator to ignore the colour.
pub fn is_blocked(decision: Decision) -> bool {
    matches!(decision, Decision::Deny)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_parses_its_unit() {
        assert_eq!(parse_window(Some("5m")).expect("5m").seconds, 300);
        assert_eq!(parse_window(Some("90s")).expect("90s").seconds, 90);
        assert_eq!(parse_window(Some("2h")).expect("2h").seconds, 7200);
        assert_eq!(parse_window(Some(" 15m ")).expect("trimmed").seconds, 900);
    }

    #[test]
    fn a_missing_window_is_five_minutes() {
        assert_eq!(parse_window(None).expect("default"), Window::default_window());
        assert_eq!(parse_window(Some("")).expect("empty"), Window::default_window());
    }

    #[test]
    fn a_window_that_is_not_a_duration_is_refused() {
        assert!(parse_window(Some("5min")).is_err());
        assert!(parse_window(Some("m")).is_err());
        assert!(parse_window(Some("0m")).is_err());
        assert!(parse_window(Some("-5m")).is_err());
        assert!(parse_window(Some("5")).is_err());
    }

    #[test]
    fn a_window_is_capped_not_rejected() {
        // Asking for a year is a clear intent; the cap protects the table
        // without turning the request into an error.
        let window = parse_window(Some("8760h")).expect("capped");
        assert_eq!(window.seconds, MAX_WINDOW_SECONDS);
        assert_eq!(window.label(), "24h");
    }

    #[test]
    fn the_label_is_canonical() {
        assert_eq!(Window { seconds: 30 }.label(), "30s");
        assert_eq!(Window { seconds: 300 }.label(), "5m");
        assert_eq!(Window { seconds: 3600 }.label(), "1h");
        assert_eq!(Window { seconds: 5400 }.label(), "90m");
    }

    #[test]
    fn the_live_window_is_shorter_than_the_row_window() {
        let window = parse_window(Some("24h")).expect("24h").capped(MAX_LIVE_WINDOW_SECONDS);
        assert_eq!(window.seconds, MAX_LIVE_WINDOW_SECONDS);
    }

    #[test]
    fn a_filter_defaults_to_everything() {
        assert_eq!(parse_filter(None), store::Filter::All);
        assert_eq!(parse_filter(Some("nonsense")), store::Filter::All);
        assert_eq!(parse_filter(Some("blocked")), store::Filter::Blocked);
        assert_eq!(parse_filter(Some("mcp")), store::Filter::Mcp);
    }

    #[test]
    fn a_page_size_is_clamped_to_the_store_maximum() {
        assert_eq!(page_size(None, 100), 100);
        assert_eq!(page_size(Some(10_000), 100), store::MAX_LIMIT);
        assert_eq!(page_size(Some(0), 100), 1);
    }

    #[test]
    fn a_malformed_cursor_starts_from_the_newest_row() {
        assert!(cursor_of(Some("not-a-cursor")).is_none());
        assert!(cursor_of(None).is_none());
    }

    #[test]
    fn only_a_full_page_has_a_next_cursor() {
        let event = |ts: DateTime<Utc>| TrafficEvent {
            id: Uuid::new_v4(),
            sandbox_id: None,
            sandbox_name: None,
            node_id: None,
            ts,
            host: "example.com".into(),
            port: 443,
            protocol: "https".into(),
            method: None,
            path: None,
            status: None,
            duration_ms: None,
            bytes_in: 0,
            bytes_out: 0,
            decision: Decision::Allow,
            reason: crate::domain::egress::DecisionReason::Rule,
            rule_id: None,
            window_id: None,
            agent_hint: None,
            mcp_session_id: None,
            mcp_method: None,
            mcp_tool: None,
            inspected: true,
            has_body: false,
            channel: crate::domain::egress::TrafficChannel::Inspected,
            listener: Some("terminated".into()),
            sni: None,
            app_protocol: None,
            country: None,
            asn: None,
            binary: None,
            reason_text: None,
        };
        let now = Utc::now();
        assert!(next_cursor(&[event(now)], 2).is_none());
        let cursor = next_cursor(&[event(now), event(now)], 2).expect("full page");
        assert!(Cursor::decode(&cursor).is_some());
    }

    #[test]
    fn a_deployed_view_explains_a_mismatch() {
        let view = deployment_view(Some(store::Deployment {
            node_id: None,
            node_name: Some("pve01".into()),
            reported_hash: Some("aaaaaaaaaaaa1111".into()),
            reported_at: None,
            compiled_hash: Some("bbbbbbbbbbbb2222".into()),
            compiled_at: None,
            rule_count: 3,
            window_count: 0,
        }));
        assert!(!view.deployed);
        assert!(view.explanation.contains("older rule set"));
        assert!(view.explanation.contains("aaaaaaaaaaaa"));
    }

    #[test]
    fn a_missing_deployment_is_not_deployed() {
        let view = deployment_view(None);
        assert!(!view.deployed);
        assert!(view.reported_hash.is_none());
    }

    #[test]
    fn a_deny_is_the_only_thing_the_map_calls_blocked() {
        assert!(is_blocked(Decision::Deny));
        assert!(!is_blocked(Decision::Allow));
        // Audit passes; colouring it as a block would teach the wrong lesson.
        assert!(!is_blocked(Decision::Audit));
    }
}
