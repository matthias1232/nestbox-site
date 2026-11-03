//! The traffic log as the console reads it: rows, aggregates, and map edges.
//!
//! Reads across `telemetry_events` (`kind = 'network'`), `traffic_bodies`,
//! `mcp_tool_calls`, `policy_checks`' companions (`network_rules`,
//! `policy_compilations`, `nodes`) for one sandbox and one window. Every query
//! carries both `environment_id` and `sandbox_id`, and every one is bounded by
//! `ts` with `LIMIT`: an unbounded scan over a week of traffic is exactly the
//! query that would take the console down with it.
//!
//! This module owns:
//!
//! * the paginated row list with filters (blocked only, allowed only, MCP only,
//!   host);
//! * the window summary the console shows above that list, so it does not count
//!   rows in the browser;
//! * the map edges aggregated per destination for the last few minutes;
//! * the environment-wide edges that `GET /topology?live=` adds to the static
//!   graph;
//! * one body, the MCP calls, and the rules that apply to a sandbox.
//!
//! ## Why the row shape is read from `detail` and not from a column per field
//!
//! The proxy's access log is stored as the JSON document it is
//! (`telemetry_events.detail`), because the fields are the proxy's and they gain
//! one at a time. Reading them here with `->>` and a numeric guard keeps a
//! malformed value a `NULL`/`0` instead of a failed cast that would blank the
//! whole page. Two spellings are accepted for the destination (`host`, the
//! access-log name, and `destination`, the name the older telemetry type used),
//! so a row written by either ingest path still appears.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::egress::{
    agent_hint, Decision, DecisionReason, OpenWindow, TrafficChannel, TrafficEdge, TrafficEvent,
    WindowChannel, WindowMode,
};
use crate::domain::mcp::McpToolCall;
use crate::domain::policy::{Enforcement, NetworkRule, RuleAction, RuleProtocol};
use crate::store::postgres::policies::read_matchers;
use crate::error::StoreError;

/// The largest page this module returns, whatever the caller asks for.
///
/// Enforced here as well as in the service: a limit is a property of the query,
/// and a caller that forgets to clamp must not be able to ask for a million
/// rows.
pub const MAX_LIMIT: i64 = 200;

/// The largest number of edges one map query may return.
pub const MAX_EDGES: i64 = 500;

/// The destination of a row, whichever ingest path wrote it.
///
/// Three spellings exist and all three are accepted: `host` from the proxy's
/// access log, `destination` from the older telemetry type, and `hostname` from
/// a DNS event, which has no connection to name. A reader that knew only one
/// would show a gap where there is none.
const HOST_EXPR: &str = r#"COALESCE(NULLIF(e.detail->>'host', ''), NULLIF(e.detail->>'destination', ''), NULLIF(e.detail->>'hostname', ''))"#;

/// The columns of one traffic row, as read from the event detail.
///
/// Built from one expression per field so the row list, the summary, and both
/// aggregate queries cannot drift apart in what they consider a request.
const COLUMNS_SQL: &str = r#"
        e.id,
        e.sandbox_id,
        e.ts,
        __HOST__ AS host,
        CASE WHEN COALESCE(e.detail->>'port', '') ~ '^[0-9]{1,5}$'
             THEN (e.detail->>'port')::integer ELSE 0 END AS port,
        COALESCE(NULLIF(e.detail->>'protocol', ''), 'tcp') AS protocol,
        NULLIF(e.detail->>'method', '') AS method,
        NULLIF(e.detail->>'path', '') AS path,
        CASE WHEN COALESCE(e.detail->>'status', '') ~ '^[0-9]{1,3}$'
             THEN (e.detail->>'status')::integer ELSE NULL END AS status,
        CASE WHEN COALESCE(e.detail->>'duration_ms', '') ~ '^[0-9]{1,12}$'
             THEN (e.detail->>'duration_ms')::bigint ELSE NULL END AS duration_ms,
        CASE WHEN COALESCE(e.detail->>'bytes_in', '') ~ '^[0-9]{1,18}$'
             THEN (e.detail->>'bytes_in')::bigint ELSE 0 END AS bytes_in,
        CASE WHEN COALESCE(e.detail->>'bytes_out', '') ~ '^[0-9]{1,18}$'
             THEN (e.detail->>'bytes_out')::bigint ELSE 0 END AS bytes_out,
        COALESCE(e.detail->>'decision', '') AS decision,
        COALESCE(e.detail->>'reason', 'rule') AS reason,
        NULLIF(e.detail->>'rule_id', '') AS rule_id,
        NULLIF(e.detail->>'window_id', '') AS window_id,
        NULLIF(e.detail->>'user_agent', '') AS user_agent,
        NULLIF(e.detail->>'mcp_session_id', '') AS mcp_session_id,
        NULLIF(e.detail->>'mcp_method', '') AS mcp_method,
        NULLIF(e.detail->>'mcp_tool', '') AS mcp_tool,
        COALESCE(e.detail->>'inspected', 'false') = 'true' AS inspected,
        -- The channel the ingest decided. A row from a proxy version that did
        -- not send one is a plain forward-proxy request, which is what every
        -- row was before the channel existed.
        COALESCE(NULLIF(e.detail->>'channel', ''), 'proxy') AS channel,
        NULLIF(e.detail->>'sni', '') AS sni,
        NULLIF(e.detail->>'listener', '') AS listener,
        NULLIF(e.detail->>'app_protocol', '') AS app_protocol,
        CASE WHEN e.detail->>'listener' = 'openshell' THEN NULLIF(e.detail->>'source', '') END AS program,
        NULLIF(e.detail->>'reason', '') AS reason_text,
        -- Where the destination's address is registered, from the registry's own
        -- RDAP record, looked up once per host and cached in `host_geography`.
        g.country AS country,
        g.asn AS asn
"#;

/// The column list with the destination expression substituted.
///
/// The SQL is a plain constant and the substitution is a `replace`, not a
/// `format!`: the regex guards contain `{1,5}`, which a format string reads as a
/// placeholder and refuses.
fn columns() -> String {
    COLUMNS_SQL.replace("__HOST__", HOST_EXPR)
}

/// The predicate every traffic read shares.
///
/// A row is traffic evidence when it is a connection *or* a name lookup for this
/// tenant, inside the window, with a decision the vocabulary knows and a
/// destination to show. Rows without a decision are not shown as traffic: they
/// would need a decision invented for them, and the console's first column is
/// the decision.
fn evidence() -> String {
    format!(
        r#"
        e.kind IN ('network', 'dns')
        AND e.ts >= $3
        AND e.ts <= $4
        AND COALESCE(e.detail->>'decision', '') IN ('allow', 'deny', 'audit')
        AND {HOST_EXPR} IS NOT NULL
"#
    )
}

/// The row predicate a filter adds, with the parameter slot left as a token.
///
/// The scoped queries bind the filter word at `$5` and the environment-wide ones
/// at `$4`, because the environment query has no sandbox parameter. One source
/// with a substituted slot is what keeps the four filter words from drifting
/// apart between the two.
const FILTER_SQL: &str = r#"(
        __F__::text = 'all'
        OR (__F__::text = 'blocked' AND decision = 'deny')
        OR (__F__::text = 'allowed' AND decision IN ('allow', 'audit'))
        OR (__F__::text = 'mcp' AND protocol = 'mcp')
    )"#;

/// The filter predicate bound at one parameter slot.
fn filter_at(slot: &str) -> String {
    FILTER_SQL.replace("__F__", slot)
}

/// One sandbox's window: the CTE every scoped query is built on.
///
/// `$1` environment, `$2` sandbox, `$3` from, `$4` to.
fn base_scoped() -> String {
    format!(
        "WITH base AS (
    SELECT {columns}
      FROM telemetry_events e
      LEFT JOIN host_geography g
             ON g.host = {HOST_EXPR}
     WHERE e.environment_id = $1
       AND e.sandbox_id = $2
       AND {evidence}
)",
        columns = columns(),
        evidence = evidence()
    )
}

/// A whole environment's window, for the cluster graph.
///
/// `$1` environment, `$2` from, `$3` to. Numbered from one because this query
/// has no sandbox parameter; a bound parameter the statement does not reference
/// is an error at prepare time, not a harmless extra.
fn base_environment() -> String {
    format!(
        "WITH base AS (
    SELECT {columns}, s.name AS sandbox_name, s.node_id AS node_id,
           COALESCE((e.detail->>'would_deny')::boolean, false) AS would_deny
      FROM telemetry_events e
      LEFT JOIN host_geography g
             ON g.host = {HOST_EXPR}
      JOIN sandboxes s ON s.id = e.sandbox_id AND s.environment_id = e.environment_id
     WHERE e.environment_id = $1
       AND e.ts >= $2
       AND e.ts <= $3
       AND e.sandbox_id IS NOT NULL
       AND e.kind IN ('network', 'dns')
       AND COALESCE(e.detail->>'decision', '') IN ('allow', 'deny', 'audit')
       AND {HOST_EXPR} IS NOT NULL
)",
        columns = columns()
    )
}

/// Which rows the traffic list keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    /// The connections the policy stopped.
    Blocked,
    /// The connections that passed, including audit-only ones.
    Allowed,
    /// The connections the proxy identified as MCP.
    Mcp,
}

impl Filter {
    /// The word the query binds.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Blocked => "blocked",
            Self::Allowed => "allowed",
            Self::Mcp => "mcp",
        }
    }
}

/// One page request for the traffic rows.
#[derive(Debug, Clone, Copy)]
pub struct RowQuery<'a> {
    pub sandbox_id: Uuid,
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub filter: Filter,
    /// A case-insensitive fragment of the destination host.
    pub host: Option<&'a str>,
    /// The last row of the previous page: its timestamp and its id.
    pub after: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Counts over one window, plus how many rows the active filter keeps.
///
/// The counts cover the whole window on purpose: they are the header of the
/// tab, and a number that changes when the operator toggles "only blocked" is a
/// number nobody can compare across two looks.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Summary {
    pub requests: i64,
    pub allowed: i64,
    pub denied: i64,
    pub audit: i64,
    pub bytes_in: i64,
    pub bytes_out: i64,
    /// Distinct destinations, counted the way the map counts edges: the same
    /// host on another port or another protocol is another destination.
    pub destinations: i64,
    pub mcp_requests: i64,
    /// Rows the active filter would return.
    pub matching: i64,
    /// Per-protocol counts, largest first.
    pub protocols: Vec<(String, i64)>,
}

/// A stored body, after redaction, with the moment it is deleted.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredBody {
    pub event_id: Uuid,
    pub ts: DateTime<Utc>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub request_bytes: i64,
    pub response_bytes: i64,
    /// True when the proxy hit its buffer limit and the body is a prefix.
    pub truncated: bool,
}

/// One edge of the environment-wide live map, with the sandbox's name and node.
#[derive(Debug, Clone)]
pub struct LiveEdge {
    pub sandbox_id: Uuid,
    pub sandbox_name: String,
    pub node_id: Option<Uuid>,
    pub edge: TrafficEdge,
}

/// Whether the node is enforcing the rule set the control API compiled.
///
/// This is the fact behind the console's deployed badge: two hashes, compared.
/// A node that has reported nothing is `None`, not "deployed".
#[derive(Debug, Clone, serde::Serialize)]
pub struct Deployment {
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
    pub reported_hash: Option<String>,
    pub reported_at: Option<DateTime<Utc>>,
    pub compiled_hash: Option<String>,
    pub compiled_at: Option<DateTime<Utc>>,
    pub rule_count: i64,
    pub window_count: i64,
}

impl Deployment {
    /// Whether the node enforces exactly the compiled rule set.
    ///
    /// Two hashes, both present, equal. Anything else — no report, no
    /// compilation, a difference — is `false`, because an unverifiable claim is
    /// not a deployed one.
    pub fn is_deployed(&self) -> bool {
        match (&self.reported_hash, &self.compiled_hash) {
            (Some(reported), Some(compiled)) => reported == compiled,
            _ => false,
        }
    }
}

/// A rule and where it came from: the policy's name and scope, or `None` for a
/// rule drafted directly for a sandbox.
pub type SourcedRule = (NetworkRule, Option<String>, Option<String>);

/// Map a decision string read from the event detail.
///
/// `None` for anything else. The SQL already restricts the set, so this is the
/// second of two controls rather than a filter the caller must remember.
fn parse_decision(value: &str) -> Option<Decision> {
    match value {
        "allow" => Some(Decision::Allow),
        "deny" => Some(Decision::Deny),
        "audit" => Some(Decision::Audit),
        _ => None,
    }
}

/// Map a reason string. An unrecognised value is reported as `rule` rather than
/// dropping the row; the decision, which is what an operator acts on, is intact.
fn parse_reason(value: &str) -> DecisionReason {
    match value {
        "window" => DecisionReason::Window,
        "default_deny" => DecisionReason::DefaultDeny,
        "backend_unavailable" => DecisionReason::BackendUnavailable,
        _ => DecisionReason::Rule,
    }
}

/// Parse a uuid stored as text in the event detail, tolerating anything else.
fn parse_uuid(value: Option<String>) -> Option<Uuid> {
    value.and_then(|raw| raw.parse::<Uuid>().ok())
}

/// Turn one base row into the traffic event the console draws.
fn row_to_event(row: &sqlx::postgres::PgRow) -> Result<Option<TrafficEvent>, StoreError> {
    let decision_raw: String = row.try_get("decision")?;
    let Some(decision) = parse_decision(&decision_raw) else {
        return Ok(None);
    };
    let reason_raw: String = row.try_get("reason")?;
    let user_agent: Option<String> = row.try_get("user_agent")?;
    let channel_raw: String = row.try_get("channel")?;

    Ok(Some(TrafficEvent {
        id: row.try_get("id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        sandbox_name: row.try_get("sandbox_name").unwrap_or(None),
        node_id: None,
        ts: row.try_get("ts")?,
        host: row.try_get("host")?,
        port: row.try_get("port")?,
        protocol: row.try_get("protocol")?,
        method: row.try_get("method")?,
        path: row.try_get("path")?,
        status: row.try_get("status")?,
        duration_ms: row.try_get("duration_ms")?,
        bytes_in: row.try_get("bytes_in")?,
        bytes_out: row.try_get("bytes_out")?,
        decision,
        reason: parse_reason(&reason_raw),
        rule_id: parse_uuid(row.try_get("rule_id")?),
        window_id: parse_uuid(row.try_get("window_id")?),
        agent_hint: agent_hint(user_agent.as_deref()),
        mcp_session_id: row.try_get("mcp_session_id")?,
        mcp_method: row.try_get("mcp_method")?,
        mcp_tool: row.try_get("mcp_tool")?,
        inspected: row.try_get("inspected")?,
        has_body: false,
        channel: TrafficChannel::parse(&channel_raw).unwrap_or(TrafficChannel::Proxy),
        listener: row.try_get("listener")?,
        sni: row.try_get("sni")?,
        app_protocol: row.try_get("app_protocol")?,
        country: row.try_get("country")?,
        asn: row.try_get("asn")?,
        binary: row.try_get("program")?,
        reason_text: row.try_get("reason_text")?,
    }))
}

/// Whether this sandbox exists in this environment.
///
/// Every read below is already scoped, so a foreign sandbox would return an
/// empty list. `404` is the honest answer: an empty page and a sandbox that is
/// not yours must not look the same.
pub async fn sandbox_exists(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<bool, StoreError> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    Ok(found.is_some())
}

/// Read one page of traffic rows, newest first.
///
/// Keyset paging on `(ts, id)`: `OFFSET` would scan and discard the rows before
/// the page, and a concurrent insert would shift rows between pages.
pub async fn rows(
    scope: &mut TenantScope<'_>,
    query: RowQuery<'_>,
) -> Result<Vec<TrafficEvent>, StoreError> {
    let sql = format!(
        "{base}
         SELECT id, sandbox_id, ts, host, port, protocol, method, path, status,
                duration_ms, bytes_in, bytes_out, decision, reason, rule_id,
                window_id, user_agent, mcp_session_id, mcp_method, mcp_tool, inspected,
                channel, sni, listener, app_protocol, program, reason_text, country, asn
           FROM base
          WHERE {filter}
            AND ($6::text IS NULL OR host ILIKE '%' || $6 || '%')
            AND ($7::timestamptz IS NULL OR (ts, id) < ($7::timestamptz, $8::uuid))
          ORDER BY ts DESC, id DESC
          LIMIT $9",
        base = base_scoped(),
        filter = filter_at("$5")
    );

    let (after_ts, after_id) = match query.after {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };

    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(query.sandbox_id)
        .bind(query.since)
        .bind(query.until)
        .bind(query.filter.as_str())
        .bind(query.host)
        .bind(after_ts)
        .bind(after_id)
        .bind(query.limit.clamp(1, MAX_LIMIT))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(event) = row_to_event(&row)? {
            out.push(event);
        }
    }
    Ok(out)
}

/// One page request for the environment-wide traffic list.
///
/// The same window and filters as the per-sandbox list, plus an optional sandbox
/// so one tab can be narrowed without a second endpoint. Every row carries the
/// sandbox's name: the environment view is the one place an operator cannot tell
/// two sandboxes apart from the id alone.
#[derive(Debug, Clone, Copy)]
pub struct EnvironmentRowQuery<'a> {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub filter: Filter,
    /// A case-insensitive fragment of the destination host.
    pub host: Option<&'a str>,
    /// Restrict to one sandbox, still tenant-scoped.
    pub sandbox_id: Option<Uuid>,
    /// The last row of the previous page: its timestamp and its id.
    pub after: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Read one page of traffic across every sandbox of the environment.
///
/// Parameters are numbered from one because `base_environment` has no sandbox
/// parameter: `$4` is the filter word, `$5` the optional sandbox.
pub async fn rows_environment(
    scope: &mut TenantScope<'_>,
    query: EnvironmentRowQuery<'_>,
) -> Result<Vec<TrafficEvent>, StoreError> {
    let sql = format!(
        "{base}
         SELECT id, sandbox_id, sandbox_name, ts, host, port, protocol, method, path, status,
                duration_ms, bytes_in, bytes_out, decision, reason, rule_id,
                window_id, user_agent, mcp_session_id, mcp_method, mcp_tool, inspected,
                channel, sni, listener, app_protocol, program, reason_text, country, asn
           FROM base
          WHERE {filter}
            AND ($5::uuid IS NULL OR sandbox_id = $5::uuid)
            AND ($6::text IS NULL OR host ILIKE '%' || $6 || '%')
            AND ($7::timestamptz IS NULL OR (ts, id) < ($7::timestamptz, $8::uuid))
          ORDER BY ts DESC, id DESC
          LIMIT $9",
        base = base_environment(),
        filter = filter_at("$4")
    );

    let (after_ts, after_id) = match query.after {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };

    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(query.since)
        .bind(query.until)
        .bind(query.filter.as_str())
        .bind(query.sandbox_id)
        .bind(query.host)
        .bind(after_ts)
        .bind(after_id)
        .bind(query.limit.clamp(1, MAX_LIMIT))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(event) = row_to_event(&row)? {
            out.push(event);
        }
    }
    Ok(out)
}

/// Count one window across every sandbox of the environment.
///
/// The header of the live console, computed by the database for the same reason
/// the per-sandbox summary is: a browser counting a five-minute window at ten
/// thousand events a minute stops responding.
pub async fn summary_environment(
    scope: &mut TenantScope<'_>,
    query: EnvironmentRowQuery<'_>,
) -> Result<Summary, StoreError> {
    let sql = format!(
        "{base}
         SELECT COUNT(*) AS requests,
                COUNT(*) FILTER (WHERE decision = 'allow') AS allowed,
                COUNT(*) FILTER (WHERE decision = 'deny') AS denied,
                COUNT(*) FILTER (WHERE decision = 'audit') AS audit,
                COALESCE(SUM(bytes_in), 0)::bigint AS bytes_in,
                COALESCE(SUM(bytes_out), 0)::bigint AS bytes_out,
                COUNT(DISTINCT (host, port, protocol)) AS destinations,
                COUNT(*) FILTER (WHERE protocol = 'mcp') AS mcp_requests,
                COUNT(*) FILTER (WHERE {filter}
                                   AND ($5::uuid IS NULL OR sandbox_id = $5::uuid)
                                   AND ($6::text IS NULL OR host ILIKE '%' || $6 || '%')) AS matching
           FROM base",
        base = base_environment(),
        filter = filter_at("$4")
    );

    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(query.since)
        .bind(query.until)
        .bind(query.filter.as_str())
        .bind(query.sandbox_id)
        .bind(query.host)
        .fetch_one(scope.connection())
        .await?;

    let mut summary = Summary {
        requests: row.try_get("requests")?,
        allowed: row.try_get("allowed")?,
        denied: row.try_get("denied")?,
        audit: row.try_get("audit")?,
        bytes_in: row.try_get("bytes_in")?,
        bytes_out: row.try_get("bytes_out")?,
        destinations: row.try_get("destinations")?,
        mcp_requests: row.try_get("mcp_requests")?,
        matching: row.try_get("matching")?,
        protocols: Vec::new(),
    };

    let facet_sql = format!(
        "{base}
         SELECT protocol, COUNT(*) AS n
           FROM base
          WHERE ($4::uuid IS NULL OR sandbox_id = $4::uuid)
            AND ($5::text IS NULL OR host ILIKE '%' || $5 || '%')
          GROUP BY protocol
          ORDER BY n DESC, protocol
          LIMIT 12",
        base = base_environment()
    );

    let facets = sqlx::query(&facet_sql)
        .bind(scope.environment_id())
        .bind(query.since)
        .bind(query.until)
        .bind(query.sandbox_id)
        .bind(query.host)
        .fetch_all(scope.connection())
        .await?;

    for facet in facets {
        summary
            .protocols
            .push((facet.try_get("protocol")?, facet.try_get("n")?));
    }

    Ok(summary)
}

/// Mark the rows of one page that have a stored body.
///
/// One query for the page rather than a correlated subquery per candidate row:
/// the body table is partitioned by day, so a per-row probe would touch every
/// partition of the page's time range.
pub async fn mark_bodies(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    rows: &mut [TrafficEvent],
) -> Result<(), StoreError> {
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let present: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT event_id
           FROM traffic_bodies
          WHERE environment_id = $1 AND sandbox_id = $2 AND event_id = ANY($3)",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(&ids)
    .fetch_all(scope.connection())
    .await?;

    let present: std::collections::HashSet<Uuid> = present.into_iter().collect();
    for row in rows.iter_mut() {
        row.has_body = present.contains(&row.id);
    }
    Ok(())
}

/// Count one window, and count what the active filter keeps.
///
/// One aggregate query and one small grouping by protocol. Both are bounded by
/// the window and the sandbox; neither returns a row per event.
pub async fn summary(
    scope: &mut TenantScope<'_>,
    query: RowQuery<'_>,
) -> Result<Summary, StoreError> {
    let sql = format!(
        "{base}
         SELECT COUNT(*) AS requests,
                COUNT(*) FILTER (WHERE decision = 'allow') AS allowed,
                COUNT(*) FILTER (WHERE decision = 'deny') AS denied,
                COUNT(*) FILTER (WHERE decision = 'audit') AS audit,
                COALESCE(SUM(bytes_in), 0)::bigint AS bytes_in,
                COALESCE(SUM(bytes_out), 0)::bigint AS bytes_out,
                COUNT(DISTINCT (host, port, protocol)) AS destinations,
                COUNT(*) FILTER (WHERE protocol = 'mcp') AS mcp_requests,
                COUNT(*) FILTER (WHERE {filter}
                                   AND ($6::text IS NULL OR host ILIKE '%' || $6 || '%')) AS matching
           FROM base",
        base = base_scoped(),
        filter = filter_at("$5")
    );

    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(query.sandbox_id)
        .bind(query.since)
        .bind(query.until)
        .bind(query.filter.as_str())
        .bind(query.host)
        .fetch_one(scope.connection())
        .await?;

    let mut summary = Summary {
        requests: row.try_get("requests")?,
        allowed: row.try_get("allowed")?,
        denied: row.try_get("denied")?,
        audit: row.try_get("audit")?,
        bytes_in: row.try_get("bytes_in")?,
        bytes_out: row.try_get("bytes_out")?,
        destinations: row.try_get("destinations")?,
        mcp_requests: row.try_get("mcp_requests")?,
        matching: row.try_get("matching")?,
        protocols: Vec::new(),
    };

    let facet_sql = format!(
        "{base}
         SELECT protocol, COUNT(*) AS n
           FROM base
          WHERE ($5::text IS NULL OR host ILIKE '%' || $5 || '%')
          GROUP BY protocol
          ORDER BY n DESC, protocol
          LIMIT 12",
        base = base_scoped()
    );

    let facets = sqlx::query(&facet_sql)
        .bind(scope.environment_id())
        .bind(query.sandbox_id)
        .bind(query.since)
        .bind(query.until)
        .bind(query.host)
        .fetch_all(scope.connection())
        .await?;

    for facet in facets {
        summary
            .protocols
            .push((facet.try_get("protocol")?, facet.try_get("n")?));
    }

    Ok(summary)
}

/// One destination that was denied — or that only got through — grouped, as the
/// decisions view lists it.
///
/// Grouped rather than one row per request: an operator decides about a
/// destination, not about the four hundred attempts it made. The counts and the
/// two timestamps are what make the decision reviewable.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeniedTarget {
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: Option<String>,
    pub host: String,
    pub port: i32,
    pub protocol: String,
    pub channel: String,
    pub denials: i64,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// The enforcement's own words for the most recent refusal.
    pub reason: Option<String>,
    /// True when nothing in the group was ever really refused: the target was
    /// only ever let through while the rules said it would be denied.
    pub observed_only: bool,
}

/// The destinations that were denied in one window, worst first.
///
/// This is the read behind the decisions view: what was stopped, grouped by the
/// thing a person can allow, so the list is short enough to act on. A target
/// that a suspended rule set let through is listed too, flagged
/// `observed_only`: the question "may this host be reached" was answered
/// "would have been no", and an operator who only sees hard denials loses the
/// list the moment somebody opens an evaluation window. The informational view
/// is the plain traffic list; this one exists because "what must I decide" is a
/// different question from "what happened".
pub async fn denied_targets(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    sandbox_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<DeniedTarget>, StoreError> {
    let sql = format!(
        "{base}
         SELECT sandbox_id, sandbox_name, host, port, protocol, channel,
                COUNT(*) AS denials,
                MIN(ts) AS first_seen,
                MAX(ts) AS last_seen,
                (array_agg(reason_text ORDER BY ts DESC))[1] AS reason,
                bool_and(decision <> 'deny') AS observed_only
           FROM base
          WHERE (decision = 'deny' OR would_deny)
            AND ($4::uuid IS NULL OR sandbox_id = $4::uuid)
          GROUP BY sandbox_id, sandbox_name, host, port, protocol, channel
          ORDER BY denials DESC, last_seen DESC
          LIMIT $5",
        base = base_environment()
    );

    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(since)
        .bind(until)
        .bind(sandbox_id)
        .bind(limit.clamp(1, MAX_EDGES))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(DeniedTarget {
            sandbox_id: row.try_get("sandbox_id")?,
            sandbox_name: row.try_get("sandbox_name")?,
            host: row.try_get("host")?,
            port: row.try_get("port")?,
            protocol: row.try_get("protocol")?,
            channel: row.try_get("channel")?,
            denials: row.try_get("denials")?,
            first_seen: row.try_get("first_seen")?,
            last_seen: row.try_get("last_seen")?,
            reason: row.try_get("reason")?,
            observed_only: row.try_get("observed_only")?,
        });
    }
    Ok(out)
}

/// Aggregate one sandbox's destinations into map edges.
///
/// Grouped in SQL rather than shipped as rows: a five-minute window at ten
/// thousand events a minute is fifty thousand rows, and a browser counting them
/// is a browser that stops responding.
pub async fn map_edges(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<TrafficEdge>, StoreError> {
    let sql = format!(
        "{base}
         SELECT host, port, protocol,
                COUNT(*) AS requests,
                COUNT(*) FILTER (WHERE decision = 'allow') AS allowed,
                COUNT(*) FILTER (WHERE decision = 'deny') AS denied,
                COUNT(*) FILTER (WHERE decision = 'audit') AS audited,
                COALESCE(SUM(bytes_in + bytes_out), 0)::bigint AS bytes,
                MAX(ts) AS last_seen,
                COALESCE(array_agg(DISTINCT mcp_tool) FILTER (WHERE mcp_tool IS NOT NULL),
                         ARRAY[]::text[]) AS tools
           FROM base
          GROUP BY host, port, protocol
          ORDER BY requests DESC, host, port
          LIMIT $5",
        base = base_scoped()
    );

    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(since)
        .bind(until)
        .bind(limit.clamp(1, MAX_EDGES))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TrafficEdge {
            sandbox_id,
            host: row.try_get("host")?,
            port: row.try_get("port")?,
            protocol: row.try_get("protocol")?,
            requests: row.try_get("requests")?,
            allowed: row.try_get("allowed")?,
            denied: row.try_get("denied")?,
            bytes: row.try_get("bytes")?,
            last_seen: row.try_get("last_seen")?,
            tools: row.try_get("tools")?,
        });
    }
    Ok(out)
}

/// Aggregate every sandbox of the environment into live edges, for the cluster
/// graph.
///
/// The static graph says what belongs to what; these edges say what is talking
/// to what right now. Ordered by volume so the busiest connections survive the
/// limit, and capped, because a graph with ten thousand edges is not a graph.
pub async fn live_edges(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<LiveEdge>, StoreError> {
    let sql = format!(
        "{base}
         SELECT sandbox_id, sandbox_name, node_id, host, port, protocol,
                COUNT(*) AS requests,
                COUNT(*) FILTER (WHERE decision = 'allow') AS allowed,
                COUNT(*) FILTER (WHERE decision = 'deny') AS denied,
                COALESCE(SUM(bytes_in + bytes_out), 0)::bigint AS bytes,
                MAX(ts) AS last_seen,
                COALESCE(array_agg(DISTINCT mcp_tool) FILTER (WHERE mcp_tool IS NOT NULL),
                         ARRAY[]::text[]) AS tools
           FROM base
          GROUP BY sandbox_id, sandbox_name, node_id, host, port, protocol
          ORDER BY requests DESC, sandbox_name, host
          LIMIT $4",
        base = base_environment()
    );

    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(since)
        .bind(until)
        .bind(limit.clamp(1, MAX_EDGES))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let sandbox_id: Uuid = row.try_get("sandbox_id")?;
        out.push(LiveEdge {
            sandbox_id,
            sandbox_name: row.try_get("sandbox_name")?,
            node_id: row.try_get("node_id")?,
            edge: TrafficEdge {
                sandbox_id,
                host: row.try_get("host")?,
                port: row.try_get("port")?,
                protocol: row.try_get("protocol")?,
                requests: row.try_get("requests")?,
                allowed: row.try_get("allowed")?,
                denied: row.try_get("denied")?,
                bytes: row.try_get("bytes")?,
                last_seen: row.try_get("last_seen")?,
                tools: row.try_get("tools")?,
            },
        });
    }
    Ok(out)
}

/// Read the stored body of one traffic row.
///
/// `None` when there is no body — either the proxy did not capture one or it has
/// passed its 24-hour retention. The caller answers 404 for both, because the
/// console's "show body" and "the body expired" are the same absence.
pub async fn body(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    event_id: Uuid,
) -> Result<Option<StoredBody>, StoreError> {
    let row = sqlx::query(
        "SELECT event_id, ts, request_body, response_body, request_bytes,
                response_bytes, truncated
           FROM traffic_bodies
          WHERE environment_id = $1 AND sandbox_id = $2 AND event_id = $3
          ORDER BY ts DESC
          LIMIT 1",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(event_id)
    .fetch_optional(scope.connection())
    .await?;

    let Some(row) = row else { return Ok(None) };
    Ok(Some(StoredBody {
        event_id: row.try_get("event_id")?,
        ts: row.try_get("ts")?,
        request_body: row.try_get("request_body")?,
        response_body: row.try_get("response_body")?,
        request_bytes: row.try_get("request_bytes")?,
        response_bytes: row.try_get("response_bytes")?,
        truncated: row.try_get("truncated")?,
    }))
}

/// One request for the MCP call list.
pub struct CallQuery<'a> {
    pub sandbox_id: Uuid,
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    /// A fragment of the server host.
    pub server_host: Option<&'a str>,
    /// An exact tool name, because a tool name is an identifier.
    pub tool: Option<&'a str>,
    pub after: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Read one page of MCP tool calls, newest first.
///
/// The arguments and result are returned as stored; the ingest path redacted
/// them before the insert, and the service redacts them again on the way out.
pub async fn mcp_calls(
    scope: &mut TenantScope<'_>,
    query: CallQuery<'_>,
) -> Result<Vec<McpToolCall>, StoreError> {
    let (after_ts, after_id) = match query.after {
        Some((ts, id)) => (Some(ts), Some(id)),
        None => (None, None),
    };

    let rows = sqlx::query(
        "SELECT id, sandbox_id, ts, server_host, server_key, session_id, method,
                tool_name, arguments, result, is_error, duration_ms, request_id
           FROM mcp_tool_calls
          WHERE environment_id = $1
            AND sandbox_id = $2
            AND ts >= $3
            AND ts <= $4
            AND ($5::text IS NULL OR server_host ILIKE '%' || $5 || '%')
            AND ($6::text IS NULL OR tool_name = $6)
            AND ($7::timestamptz IS NULL OR (ts, id) < ($7::timestamptz, $8::uuid))
          ORDER BY ts DESC, id DESC
          LIMIT $9",
    )
    .bind(scope.environment_id())
    .bind(query.sandbox_id)
    .bind(query.since)
    .bind(query.until)
    .bind(query.server_host)
    .bind(query.tool)
    .bind(after_ts)
    .bind(after_id)
    .bind(query.limit.clamp(1, MAX_LIMIT))
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(McpToolCall {
            id: row.try_get("id")?,
            sandbox_id: row.try_get("sandbox_id")?,
            ts: row.try_get("ts")?,
            server_host: row.try_get("server_host")?,
            server_key: row.try_get("server_key")?,
            session_id: row.try_get("session_id")?,
            method: row.try_get("method")?,
            tool_name: row.try_get("tool_name")?,
            arguments: row.try_get("arguments")?,
            result: row.try_get("result")?,
            is_error: row.try_get("is_error")?,
            duration_ms: row.try_get("duration_ms")?,
            request_id: row.try_get("request_id")?,
        });
    }
    Ok(out)
}

/// The rules that apply to one sandbox, with the policy they came from.
///
/// The predicate is the decision function's, not a broader one: enabled rules
/// whose `sandbox_id` is this sandbox or null (environment-wide). A rule listed
/// here is a rule the proxy will apply, which is the only thing the Policies tab
/// is allowed to claim.
pub async fn effective_rules(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    limit: i64,
) -> Result<Vec<SourcedRule>, StoreError> {
    let rows = sqlx::query(
        "SELECT r.id, r.environment_id, r.policy_id, r.sandbox_id, r.name,
                r.action, r.host, r.port, r.protocol, r.methods, r.binaries,
                r.matchers, r.enforcement, r.priority, r.enabled, r.version,
                p.name AS policy_name, p.scope AS policy_scope
           FROM network_rules r
           LEFT JOIN policies p ON p.id = r.policy_id
          WHERE r.environment_id = $1
            AND r.enabled
            AND (r.sandbox_id = $2 OR r.sandbox_id IS NULL)
          ORDER BY r.priority ASC, r.host ASC, r.name ASC
          LIMIT $3",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(limit.clamp(1, MAX_LIMIT))
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let action_raw: String = row.try_get("action")?;
        let protocol_raw: String = row.try_get("protocol")?;
        let enforcement_raw: String = row.try_get("enforcement")?;

        // The database's CHECK constraints cover these three columns, so a
        // value outside the vocabulary means a schema drift, not bad input.
        // Skipping the row is better than guessing an action.
        let (Some(action), Some(protocol), Some(enforcement)) = (
            RuleAction::parse(&action_raw),
            RuleProtocol::parse(&protocol_raw),
            Enforcement::parse(&enforcement_raw),
        ) else {
            continue;
        };

        out.push((
            NetworkRule {
                id: row.try_get("id")?,
                environment_id: row.try_get("environment_id")?,
                policy_id: row.try_get("policy_id")?,
                sandbox_id: row.try_get("sandbox_id")?,
                name: row.try_get("name")?,
                action,
                host: row.try_get("host")?,
                port: row.try_get("port")?,
                protocol,
                methods: row.try_get("methods")?,
                binaries: row.try_get("binaries")?,
                matchers: read_matchers(&row)?,
                enforcement,
                priority: row.try_get("priority")?,
                enabled: row.try_get("enabled")?,
                version: row.try_get("version")?,
            },
            row.try_get("policy_name")?,
            row.try_get("policy_scope")?,
        ));
    }
    Ok(out)
}

/// The open "allow everything" windows that cover this sandbox.
///
/// A window with no sandbox covers the whole environment; both are returned
/// because both are in force.
pub async fn open_windows(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<OpenWindow>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, sandbox_id, expires_at, reason, mode, channel
           FROM traffic_windows
          WHERE environment_id = $1
            AND (sandbox_id = $2 OR sandbox_id IS NULL)
            AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > now())
          ORDER BY expires_at ASC
          LIMIT 20",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mode: String = row.try_get("mode")?;
        let channel: String = row.try_get("channel")?;
        out.push(OpenWindow {
            id: row.try_get("id")?,
            sandbox_id: row.try_get("sandbox_id")?,
            expires_at: row.try_get("expires_at")?,
            reason: row.try_get("reason")?,
            mode: WindowMode::parse(&mode).unwrap_or_default(),
            channel: WindowChannel::parse(&channel).unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The sandbox's node, and the two hashes that decide "deployed".
///
/// A sandbox always has a node (the column is not null), but the node row and
/// the compilation may both be absent in a fresh environment; both are optional
/// here rather than defaulted to a value that would read as success.
pub async fn deployment(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<Deployment>, StoreError> {
    let row = sqlx::query(
        "SELECT s.node_id, n.name AS node_name,
                n.policy_hash_reported, n.policy_hash_reported_at,
                c.hash AS compiled_hash, c.compiled_at,
                c.rule_count::bigint AS rule_count,
                c.window_count::bigint AS window_count
           FROM sandboxes s
           LEFT JOIN nodes n ON n.id = s.node_id
           LEFT JOIN policy_compilations c ON c.environment_id = s.environment_id
          WHERE s.environment_id = $1 AND s.id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;

    let Some(row) = row else { return Ok(None) };
    Ok(Some(Deployment {
        node_id: row.try_get("node_id")?,
        node_name: row.try_get("node_name")?,
        reported_hash: row.try_get("policy_hash_reported")?,
        reported_at: row.try_get("policy_hash_reported_at")?,
        compiled_hash: row.try_get("compiled_hash")?,
        compiled_at: row.try_get("compiled_at")?,
        rule_count: row.try_get::<Option<i64>, _>("rule_count")?.unwrap_or(0),
        window_count: row.try_get::<Option<i64>, _>("window_count")?.unwrap_or(0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_words_are_the_ones_the_query_understands() {
        // The SQL compares against these literals; a rename without this test
        // would silently return every row for "blocked".
        assert_eq!(Filter::All.as_str(), "all");
        assert_eq!(Filter::Blocked.as_str(), "blocked");
        assert_eq!(Filter::Allowed.as_str(), "allowed");
        assert_eq!(Filter::Mcp.as_str(), "mcp");
    }

    #[test]
    fn an_unknown_decision_is_not_shown_as_a_decision() {
        assert!(parse_decision("maybe").is_none());
        assert_eq!(parse_decision("deny"), Some(Decision::Deny));
    }

    #[test]
    fn an_unknown_reason_still_yields_a_row() {
        assert_eq!(parse_reason("nonsense"), DecisionReason::Rule);
        assert_eq!(parse_reason("default_deny"), DecisionReason::DefaultDeny);
    }

    #[test]
    fn a_garbage_rule_id_is_dropped_rather_than_failing_the_row() {
        assert!(parse_uuid(Some("not-a-uuid".into())).is_none());
        assert!(parse_uuid(None).is_none());
        let id = Uuid::new_v4();
        assert_eq!(parse_uuid(Some(id.to_string())), Some(id));
    }

    #[test]
    fn deployed_needs_two_equal_hashes() {
        let mut deployment = Deployment {
            node_id: None,
            node_name: None,
            reported_hash: Some("abc".into()),
            reported_at: None,
            compiled_hash: Some("abc".into()),
            compiled_at: None,
            rule_count: 0,
            window_count: 0,
        };
        assert!(deployment.is_deployed());
        deployment.compiled_hash = Some("def".into());
        assert!(!deployment.is_deployed());
        deployment.compiled_hash = None;
        deployment.reported_hash = None;
        assert!(!deployment.is_deployed());
    }

    #[test]
    fn the_scoped_query_filters_on_tenant_and_sandbox() {
        let sql = base_scoped();
        assert!(sql.contains("e.environment_id = $1"));
        assert!(sql.contains("e.sandbox_id = $2"));
    }

    #[test]
    fn a_name_lookup_is_evidence_and_names_its_host() {
        // The regression this test exists for: DNS decisions used to be written
        // as `network` events with a protocol word, so the DNS facet was always
        // empty and a DNS destination had no host to show.
        let sql = base_scoped();
        assert!(sql.contains("e.kind IN ('network', 'dns')"));
        assert!(sql.contains("hostname"), "a DNS row must contribute its host");
        assert!(
            !sql.contains("__HOST__"),
            "the destination expression must be substituted, not left as a token"
        );
    }

    #[test]
    fn the_environment_query_names_its_sandbox() {
        let sql = base_environment();
        assert!(sql.contains("s.name AS sandbox_name"));
        assert!(sql.contains("e.kind IN ('network', 'dns')"));
    }

    #[test]
    fn the_filter_word_is_bound_at_the_slot_the_query_uses() {
        // The scoped queries bind the filter at $5 and the environment ones at
        // $4, because the environment query has no sandbox parameter. One source
        // with a substituted slot keeps the four words from drifting apart.
        assert!(filter_at("$5").contains("$5::text = 'all'"));
        assert!(filter_at("$4").contains("$4::text = 'blocked'"));
        assert!(!filter_at("$4").contains("__F__"));
    }

    #[test]
    fn the_environment_query_has_no_sandbox_parameter() {
        // A bound parameter the statement does not reference is a prepare-time
        // error, so the numbering has to start over for this query.
        let sql = base_environment();
        assert!(sql.contains("e.sandbox_id IS NOT NULL"));
        assert!(!sql.contains("$4"));
    }
}
