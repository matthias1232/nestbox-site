//! Rows of `chat_turns`: insert, query, group.
//!
//! The statements live here so the service has no SQL and the store has no rules.
//! Every query filters by `environment_id` first: that is the tenant boundary and
//! the first index column.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use crate::error::StoreError;
use crate::domain::privacy::{Detector, IncidentAction, PrivacyIncident};
use crate::services::chat_service::{ChatGroup, ChatTurn, NewTurn};
use crate::store::TenantScope;

/// How a set of turns is grouped. A closed list: the name becomes SQL text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    Session,
    Model,
    Provider,
    Day,
    Sandbox,
    Owner,
}

impl GroupBy {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "session" => Self::Session,
            "model" => Self::Model,
            "provider" => Self::Provider,
            "day" => Self::Day,
            "sandbox" => Self::Sandbox,
            "owner" => Self::Owner,
            _ => return None,
        })
    }

    fn key_expression(self) -> &'static str {
        match self {
            Self::Session => "coalesce(t.session_id, '(no session)')",
            Self::Model => "coalesce(t.served_model, t.requested_model, '(unknown)')",
            Self::Provider => "t.provider",
            Self::Day => "to_char(t.ts AT TIME ZONE 'UTC', 'YYYY-MM-DD')",
            Self::Sandbox => "coalesce(s.name, '(deleted)')",
            Self::Owner => "coalesce(u.display_name, '(unknown)')",
        }
    }
}

/// The filters a query may carry. One struct, so the SQL and the API agree.
#[derive(Debug, Clone, Default)]
pub struct TurnFilter<'a> {
    pub sandbox_id: Option<Uuid>,
    pub session_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub provider: Option<&'a str>,
    /// `clean` means "no finding"; anything else is compared to `chat_turns.dlp`.
    pub dlp: Option<&'a str>,
    pub account_kind: Option<&'a str>,
    pub owner: Option<Uuid>,
    pub q: Option<&'a str>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    /// Forces one owner's rows, for a non-admin. `None` for an admin.
    pub only_owner: Option<Uuid>,
    pub limit: i64,
}

const TURN_COLUMNS: &str = "t.id, t.environment_id, t.sandbox_id, t.owner_user_id, t.ts, \
    t.ended_at, t.request_ref, t.session_id, t.agent, t.provider, t.host, t.upstream_ip, \
    t.upstream_ip_would, t.account_kind, t.requested_model, t.served_model, t.router_provider, \
    t.router_model, t.user_text, t.assistant_text, t.tool_calls, t.input_tokens, t.output_tokens, \
    t.cache_read, t.cache_write, t.status, t.dlp, t.delivered, t.route, s.name AS sandbox_name, \
    u.display_name AS owner_name";

const FROM_TURNS: &str = "FROM chat_turns t \
    LEFT JOIN sandboxes s ON s.id = t.sandbox_id \
    LEFT JOIN users u ON u.id = t.owner_user_id";

/// Insert one turn. `true` when the row was new, so a retried batch does not
/// announce the same turn twice.
pub async fn insert(scope: &mut TenantScope<'_>, turn: &NewTurn<'_>) -> Result<bool, StoreError> {
    let result = sqlx::query(
        "INSERT INTO chat_turns (id, environment_id, sandbox_id, owner_user_id, ts, ended_at, \
            request_ref, session_id, agent, provider, host, upstream_ip, upstream_ip_would, \
            account_kind, requested_model, served_model, router_provider, router_model, \
            user_text, assistant_text, tool_calls, input_tokens, output_tokens, cache_read, \
            cache_write, status, dlp, delivered, route) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, \
            $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(turn.id)
    .bind(turn.environment_id)
    .bind(turn.sandbox_id)
    .bind(turn.owner_user_id)
    .bind(turn.ts)
    .bind(turn.ended_at)
    .bind(turn.request_ref)
    .bind(turn.session_id)
    .bind(turn.agent)
    .bind(turn.provider)
    .bind(turn.host)
    .bind(turn.upstream_ip)
    .bind(turn.upstream_ip_would)
    .bind(turn.account_kind)
    .bind(turn.requested_model)
    .bind(turn.served_model)
    .bind(turn.router_provider)
    .bind(turn.router_model)
    .bind(turn.user_text)
    .bind(turn.assistant_text)
    .bind(turn.tool_calls)
    .bind(turn.input_tokens)
    .bind(turn.output_tokens)
    .bind(turn.cache_read)
    .bind(turn.cache_write)
    .bind(turn.status)
    .bind(turn.dlp)
    .bind(turn.delivered)
    .bind(turn.route)
    .execute(scope.connection())
    .await?;
    Ok(result.rows_affected() > 0)
}

/// The owner and name of one sandbox, for the row's denormalised columns.
pub async fn owner_of(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Option<(Option<Uuid>, String)>, StoreError> {
    let row: Option<(Option<Uuid>, String)> = sqlx::query_as(
        "SELECT owner_user_id, name FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await?;
    Ok(row)
}

/// The turns a filter describes, newest first.
pub async fn query(
    scope: &mut TenantScope<'_>,
    filter: &TurnFilter<'_>,
) -> Result<Vec<ChatTurn>, StoreError> {
    let statement = format!(
        "SELECT {TURN_COLUMNS} {FROM_TURNS} \
          WHERE t.environment_id = $1 \
            AND ($2::uuid IS NULL OR t.sandbox_id = $2) \
            AND ($3::text IS NULL OR t.session_id = $3) \
            AND ($4::text IS NULL OR t.requested_model = $4 OR t.served_model = $4) \
            AND ($5::text IS NULL OR t.provider = $5) \
            AND ($6::text IS NULL OR ($6 = 'clean' AND t.dlp IS NULL) OR t.dlp = $6) \
            AND ($7::text IS NULL OR t.account_kind = $7) \
            AND ($8::uuid IS NULL OR t.owner_user_id = $8) \
            AND ($9::text IS NULL OR t.user_text ILIKE '%' || $9 || '%' \
                 OR t.assistant_text ILIKE '%' || $9 || '%' \
                 OR coalesce(t.session_id, '') ILIKE '%' || $9 || '%' \
                 OR coalesce(t.requested_model, '') ILIKE '%' || $9 || '%' \
                 OR coalesce(t.served_model, '') ILIKE '%' || $9 || '%' \
                 OR coalesce(t.host, '') ILIKE '%' || $9 || '%') \
            AND ($10::timestamptz IS NULL OR t.ts >= $10) \
            AND ($11::timestamptz IS NULL OR t.ts <= $11) \
            AND ($12::timestamptz IS NULL OR t.ts < $12) \
            AND ($13::uuid IS NULL OR t.owner_user_id = $13) \
          ORDER BY t.ts DESC LIMIT $14"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(filter.sandbox_id)
        .bind(filter.session_id)
        .bind(filter.model)
        .bind(filter.provider)
        .bind(filter.dlp)
        .bind(filter.account_kind)
        .bind(filter.owner)
        .bind(filter.q)
        .bind(filter.since)
        .bind(filter.until)
        .bind(filter.before)
        .bind(filter.only_owner)
        .bind(filter.limit.clamp(1, 500))
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(turn_from_row(&row)?);
    }
    Ok(out)
}

/// The same rows, grouped.
pub async fn groups(
    scope: &mut TenantScope<'_>,
    group: GroupBy,
    filter: &TurnFilter<'_>,
) -> Result<Vec<ChatGroup>, StoreError> {
    let key = group.key_expression();
    let statement = format!(
        "SELECT {key} AS key, count(*)::bigint AS turns, min(t.ts) AS first_ts, \
            max(t.ts) AS last_ts, \
            coalesce(sum(coalesce(t.input_tokens, 0)), 0)::bigint AS input_tokens, \
            coalesce(sum(coalesce(t.output_tokens, 0)), 0)::bigint AS output_tokens, \
            count(*) FILTER (WHERE t.dlp = 'blocked')::bigint AS blocked, \
            count(*) FILTER (WHERE t.dlp = 'replaced')::bigint AS replaced, \
            count(*) FILTER (WHERE t.dlp = 'pseudonymized')::bigint AS pseudonymized \
           {FROM_TURNS} \
          WHERE t.environment_id = $1 \
            AND ($2::uuid IS NULL OR t.sandbox_id = $2) \
            AND ($3::text IS NULL OR t.provider = $3) \
            AND ($4::timestamptz IS NULL OR t.ts >= $4) \
            AND ($5::text IS NULL OR t.user_text ILIKE '%' || $5 || '%' \
                 OR t.assistant_text ILIKE '%' || $5 || '%') \
            AND ($6::uuid IS NULL OR t.owner_user_id = $6) \
          GROUP BY 1 ORDER BY 4 DESC LIMIT 500"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(filter.sandbox_id)
        .bind(filter.provider)
        .bind(filter.since)
        .bind(filter.q)
        .bind(filter.only_owner)
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let key: String = row.try_get("key")?;
        out.push(ChatGroup {
            label: key.clone(),
            key,
            turns: row.try_get("turns")?,
            first_ts: row.try_get("first_ts")?,
            last_ts: row.try_get("last_ts")?,
            input_tokens: row.try_get("input_tokens")?,
            output_tokens: row.try_get("output_tokens")?,
            blocked: row.try_get("blocked")?,
            replaced: row.try_get("replaced")?,
            pseudonymized: row.try_get("pseudonymized")?,
        });
    }
    Ok(out)
}

/// One turn by id, restricted to what the caller may see.
pub async fn get(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    only_owner: Option<Uuid>,
) -> Result<Option<ChatTurn>, StoreError> {
    let statement = format!(
        "SELECT {TURN_COLUMNS} {FROM_TURNS} \
          WHERE t.environment_id = $1 AND t.id = $2 \
            AND ($3::uuid IS NULL OR t.owner_user_id = $3)"
    );
    let row = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(id)
        .bind(only_owner)
        .fetch_optional(scope.connection())
        .await?;
    match row {
        None => Ok(None),
        Some(row) => Ok(Some(turn_from_row(&row)?)),
    }
}

fn turn_from_row(row: &sqlx::postgres::PgRow) -> Result<ChatTurn, StoreError> {
    Ok(ChatTurn {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        sandbox_name: row.try_get("sandbox_name")?,
        owner_user_id: row.try_get("owner_user_id")?,
        owner_name: row.try_get("owner_name")?,
        ts: row.try_get("ts")?,
        ended_at: row.try_get("ended_at")?,
        request_ref: row.try_get("request_ref")?,
        session_id: row.try_get("session_id")?,
        agent: row.try_get("agent")?,
        provider: row.try_get("provider")?,
        host: row.try_get("host")?,
        upstream_ip: row.try_get("upstream_ip")?,
        upstream_ip_would: row.try_get("upstream_ip_would")?,
        account_kind: row.try_get("account_kind")?,
        requested_model: row.try_get("requested_model")?,
        served_model: row.try_get("served_model")?,
        router_provider: row.try_get("router_provider")?,
        router_model: row.try_get("router_model")?,
        user_text: row.try_get("user_text")?,
        assistant_text: row.try_get("assistant_text")?,
        tool_calls: row.try_get("tool_calls")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        cache_read: row.try_get("cache_read")?,
        cache_write: row.try_get("cache_write")?,
        status: row.try_get("status")?,
        dlp: row.try_get("dlp")?,
        delivered: row.try_get("delivered")?,
        route: row.try_get("route")?,
    })
}

const INCIDENT_COLUMNS: &str = "id, environment_id, sandbox_id, detector, action, severity, \
    masked_value, value_fingerprint, destination, cloud_delivery, direction, requested_action, \
    mode, replacement, source, method, path, model, mcp_tool, conversation_id, occurrences, \
    last_seen_at, request_ref, exception_id, created_at";

/// The findings recorded for one request, newest first.
///
/// A turn and its findings share the proxy's request id, which is the only join
/// that survives a masked body: the text is rewritten, the id is not.
pub async fn incidents_for_request(
    scope: &mut TenantScope<'_>,
    request_ref: &str,
) -> Result<Vec<PrivacyIncident>, StoreError> {
    let statement = format!(
        "SELECT {INCIDENT_COLUMNS} FROM privacy_incidents \
          WHERE environment_id = $1 AND request_ref = $2 \
          ORDER BY created_at DESC LIMIT 100"
    );
    let rows = sqlx::query(&statement)
        .bind(scope.environment_id())
        .bind(request_ref)
        .fetch_all(scope.connection())
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(incident_from_row(&row)?);
    }
    Ok(out)
}

fn incident_from_row(row: &sqlx::postgres::PgRow) -> Result<PrivacyIncident, StoreError> {
    let detector: String = row.try_get("detector")?;
    let detector = Detector::parse(&detector).ok_or_else(|| {
        StoreError::UnexpectedData(format!("unknown detector: {detector}"))
    })?;
    let action: String = row.try_get("action")?;
    let action = IncidentAction::parse(&action).ok_or_else(|| {
        StoreError::UnexpectedData(format!("unknown incident action: {action}"))
    })?;
    let requested: Option<String> = row.try_get("requested_action")?;
    let requested_action = match requested.as_deref() {
        Some(value) => Some(IncidentAction::parse(value).ok_or_else(|| {
            StoreError::UnexpectedData(format!("unknown requested action: {value}"))
        })?),
        None => None,
    };
    Ok(PrivacyIncident {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        sandbox_id: row.try_get("sandbox_id")?,
        detector,
        action,
        severity: row.try_get("severity")?,
        masked_value: row.try_get("masked_value")?,
        value_fingerprint: row.try_get("value_fingerprint")?,
        destination: row.try_get("destination")?,
        cloud_delivery: row.try_get("cloud_delivery")?,
        direction: row.try_get("direction")?,
        requested_action,
        mode: row.try_get("mode")?,
        replacement: row.try_get("replacement")?,
        source: row.try_get("source")?,
        method: row.try_get("method")?,
        path: row.try_get("path")?,
        model: row.try_get("model")?,
        mcp_tool: row.try_get("mcp_tool")?,
        conversation_id: row.try_get("conversation_id")?,
        occurrences: row.try_get("occurrences")?,
        last_seen_at: row.try_get("last_seen_at")?,
        request_ref: row.try_get("request_ref").ok().flatten(),
        exception_id: row.try_get("exception_id").ok().flatten(),
        created_at: row.try_get("created_at")?,
    })
}
