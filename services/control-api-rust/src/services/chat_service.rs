//! Chat turns: the rows the console reads, the queries it filters with, and who
//! may see the text.
//!
//! A turn is one model call: what was asked (masked), what answered (masked), how
//! it was routed, what the content check did, and what it cost. `route` is the
//! chain the console draws.
//!
//! This module owns the rules, not the SQL: the statements live in
//! `store::postgres::chat_turns`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::chat_turn::{agent_from_user_agent, is_model_call, provider_for, turn_from};
use crate::domain::privacy::PrivacyIncident;
use crate::domain::telemetry::LiveEvent;
use crate::error::ApiError;
use crate::services::ServiceContext;
use crate::store::TenantScope;
use crate::store::postgres::chat_turns::{self, GroupBy, TurnFilter};
use crate::domain::auth::{Permission, Principal, Role};

/// One captured model call as the ingest hands it over. Bodies are masked.
#[derive(Debug, Clone, Default)]
pub struct TurnInput {
    pub id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub request_ref: Option<String>,
    pub method: String,
    pub path: String,
    pub host: String,
    pub user_agent: Option<String>,
    pub upstream_ip: Option<String>,
    pub upstream_ip_would: Option<String>,
    pub account_kind: Option<String>,
    pub router_provider: Option<String>,
    pub router_model: Option<String>,
    pub status: Option<i32>,
    pub dlp: Option<String>,
    pub channel: Option<String>,
    pub decision: Option<String>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
}

/// Filters of the chat list. `by` is only read by [`groups`].
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct TurnQuery {
    pub sandbox_id: Option<Uuid>,
    pub session_id: Option<String>,
    /// Matches the requested or the served model.
    pub model: Option<String>,
    pub provider: Option<String>,
    /// `clean` means "no finding".
    pub dlp: Option<String>,
    pub account_kind: Option<String>,
    pub owner: Option<Uuid>,
    /// Free text over the turn and its metadata.
    pub q: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    /// Older than this timestamp: the next page.
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    /// `session`, `model`, `provider`, `day`, `sandbox` or `owner`.
    pub by: Option<String>,
}

/// One stored turn as the console reads it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChatTurn {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: Option<String>,
    pub owner_user_id: Option<Uuid>,
    pub owner_name: Option<String>,
    pub ts: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub request_ref: Option<String>,
    pub session_id: Option<String>,
    pub agent: Option<String>,
    pub provider: String,
    pub host: Option<String>,
    pub upstream_ip: Option<String>,
    pub upstream_ip_would: Option<String>,
    pub account_kind: Option<String>,
    pub requested_model: Option<String>,
    pub served_model: Option<String>,
    pub router_provider: Option<String>,
    pub router_model: Option<String>,
    pub user_text: Option<String>,
    pub assistant_text: Option<String>,
    #[schema(value_type = Object)]
    pub tool_calls: serde_json::Value,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub status: Option<i32>,
    pub dlp: Option<String>,
    pub delivered: bool,
    #[schema(value_type = Object)]
    pub route: serde_json::Value,
}

/// One group of the chat list.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChatGroup {
    pub key: String,
    pub label: String,
    pub turns: i64,
    pub first_ts: DateTime<Utc>,
    pub last_ts: DateTime<Utc>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub blocked: i64,
    pub replaced: i64,
    pub pseudonymized: i64,
}

/// A turn with the findings of its request.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChatTurnDetail {
    pub turn: ChatTurn,
    pub incidents: Vec<PrivacyIncident>,
}

/// The route a request took, as the console draws it: sandbox, proxy, content
/// check, upstream address, router, model. Hops after a refusal are marked
/// `would`, because they never happened.
pub fn build_route(
    sandbox: &str,
    channel: Option<&str>,
    decision: Option<&str>,
    dlp: Option<&str>,
    host: &str,
    ip: Option<&str>,
    ip_would: Option<&str>,
    router: Option<(&str, &str)>,
    model: Option<&str>,
) -> (serde_json::Value, bool) {
    let blocked_by_proxy = decision == Some("deny");
    let blocked_by_check = matches!(dlp, Some("blocked") | Some("unavailable") | Some("too_large"));
    let delivered = !blocked_by_proxy && !blocked_by_check;

    let mut hops: Vec<serde_json::Value> = Vec::new();
    let mut after_block = false;
    let mut push = |kind: &str, label: String, detail: String, blocked: bool| {
        let state = if after_block {
            "would"
        } else if blocked {
            after_block = true;
            "blocked"
        } else {
            "passed"
        };
        hops.push(serde_json::json!({
            "kind": kind,
            "label": label,
            "detail": detail,
            "state": state,
        }));
    };

    push("sandbox", sandbox.to_string(), String::new(), false);
    let proxy_label = match channel {
        Some("inspected") => "Egress (TLS gelesen)",
        Some("tunnel") => "Egress (Tunnel)",
        _ => "Egress",
    };
    push(
        "egress",
        proxy_label.to_string(),
        decision.unwrap_or("").to_string(),
        blocked_by_proxy,
    );
    push(
        "dlp",
        match dlp {
            Some("blocked") => "DLP: blockiert".to_string(),
            Some("replaced") => "DLP: ersetzt".to_string(),
            Some("pseudonymized") => "DLP: pseudonymisiert".to_string(),
            Some("unavailable") => "DLP: nicht geprüft".to_string(),
            Some("too_large") => "DLP: zu groß".to_string(),
            _ => "DLP: sauber".to_string(),
        },
        dlp.unwrap_or("").to_string(),
        blocked_by_check,
    );
    let upstream_detail = match ip {
        Some(ip) => format!("{host} ({ip})"),
        None => host.to_string(),
    };
    let would_detail = match ip_would {
        Some(ip) => format!("{host} ({ip})"),
        None => host.to_string(),
    };
    push(
        "upstream",
        if delivered { upstream_detail } else { would_detail },
        String::new(),
        false,
    );
    if let Some((provider, served)) = router {
        push(
            "router",
            format!("Router: {provider}"),
            served.to_string(),
            false,
        );
    }
    if let Some(model) = model {
        push("model", model.to_string(), String::new(), false);
    }

    (serde_json::Value::Array(hops), delivered)
}

/// Whether a principal owns a sandbox, for the `admin_chat_view` rule.
pub(crate) async fn owns(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
) -> bool {
    let Some(sandbox_id) = sandbox_id else {
        return false;
    };
    let owner: Option<Option<Uuid>> = sqlx::query_scalar(
        "SELECT owner_user_id FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await
    .ok()
    .flatten();
    owner.flatten() == Some(principal.user_id)
}

/// Whether the environment lets administrators read chat content.
pub(crate) async fn admin_may_read(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<bool, ApiError> {
    let settings =
        crate::services::privacy_service::effective_settings(ctx, principal, None).await?;
    Ok(settings.admin_chat_view)
}

/// Store the turns of one telemetry batch. Returns how many rows were written.
///
/// A model call is one row. The texts are the masked ones the ingest already
/// holds, so nothing here can reintroduce a value.
pub async fn record_turns(
    ctx: &ServiceContext,
    environment_id: Uuid,
    items: &[TurnInput],
) -> Result<u64, ApiError> {
    let mut scope = crate::store::TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let mut written = 0u64;
    for item in items {
        if !is_model_call(&item.method, &item.path) {
            continue;
        }
        let facts = turn_from(
            item.request_body.as_deref().unwrap_or_default(),
            item.response_body.as_deref().unwrap_or_default(),
        );
        let owner = match item.sandbox_id {
            Some(sandbox_id) => chat_turns::owner_of(&mut scope, sandbox_id)
                .await
                .map_err(ApiError::from)?,
            None => None,
        };
        let (owner_user_id, sandbox_name) = match owner {
            Some((owner, name)) => (owner, Some(name)),
            None => (None, None),
        };
        let provider = provider_for(&item.host);
        let router = match (item.router_provider.as_deref(), item.router_model.as_deref()) {
            (Some(provider), Some(model)) => Some((provider, model)),
            _ => None,
        };
        let (route, delivered) = build_route(
            sandbox_name.as_deref().unwrap_or("sandbox"),
            item.channel.as_deref(),
            item.decision.as_deref(),
            item.dlp.as_deref(),
            &item.host,
            item.upstream_ip.as_deref(),
            item.upstream_ip_would.as_deref(),
            router,
            facts
                .served_model
                .as_deref()
                .or(facts.requested_model.as_deref()),
        );
        let tool_calls = serde_json::to_value(&facts.tool_calls).unwrap_or(serde_json::json!([]));
        let turn = NewTurn {
            id: item.id,
            environment_id,
            sandbox_id: item.sandbox_id,
            owner_user_id,
            ts: item.ts,
            ended_at: item.ended_at,
            request_ref: item.request_ref.as_deref(),
            session_id: facts.session_id.as_deref(),
            agent: item
                .user_agent
                .as_deref()
                .map(agent_from_user_agent)
                .filter(|agent| *agent != "unknown"),
            provider,
            host: Some(item.host.as_str()),
            upstream_ip: item.upstream_ip.as_deref(),
            upstream_ip_would: item.upstream_ip_would.as_deref(),
            account_kind: item.account_kind.as_deref(),
            requested_model: facts.requested_model.as_deref(),
            served_model: facts.served_model.as_deref(),
            router_provider: item.router_provider.as_deref(),
            router_model: item.router_model.as_deref(),
            user_text: Some(facts.user_text.as_str()).filter(|text| !text.is_empty()),
            assistant_text: Some(facts.assistant_text.as_str()).filter(|text| !text.is_empty()),
            tool_calls: &tool_calls,
            input_tokens: facts.input_tokens,
            output_tokens: facts.output_tokens,
            cache_read: facts.cache_read,
            cache_write: facts.cache_write,
            status: item.status,
            dlp: item.dlp.as_deref(),
            delivered,
            route: &route,
        };
        if chat_turns::insert(&mut scope, &turn)
            .await
            .map_err(ApiError::from)?
        {
            written += 1;
            ctx.bus.publish(LiveEvent::ChatDone {
                environment_id,
                sandbox_id: item.sandbox_id,
                turn_id: item.id,
                request_ref: item.request_ref.clone(),
            });
        }
    }
    scope.commit().await.map_err(ApiError::from)?;
    Ok(written)
}

/// The turn list, filtered and paged.
pub async fn turns(
    ctx: &ServiceContext,
    principal: &Principal,
    query: TurnQuery,
) -> Result<Vec<ChatTurn>, ApiError> {
    crate::services::require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let filter = filter_for(&query, principal);
    let mut rows = chat_turns::query(&mut scope, &filter)
        .await
        .map_err(ApiError::from)?;
    let hide = hides_text(ctx, principal, &rows).await?;
    if hide {
        for row in &mut rows {
            hide_text(row);
        }
    }
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// The same rows, grouped.
pub async fn groups(
    ctx: &ServiceContext,
    principal: &Principal,
    query: TurnQuery,
) -> Result<Vec<ChatGroup>, ApiError> {
    crate::services::require(principal, Permission::SandboxRead)?;
    let group = query
        .by
        .as_deref()
        .and_then(GroupBy::parse)
        .ok_or_else(|| {
            ApiError::validation("by is session, model, provider, day, sandbox or owner.")
        })?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let filter = filter_for(&query, principal);
    let rows = chat_turns::groups(&mut scope, group, &filter)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// One turn with its findings.
pub async fn turn(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<ChatTurnDetail, ApiError> {
    crate::services::require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let only_owner = (principal.role != Role::Admin).then_some(principal.user_id);
    let mut found = chat_turns::get(&mut scope, id, only_owner)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::not_found(crate::error::ErrorCode::NOT_FOUND, "No such turn.")
        })?;
    let incidents = match found.request_ref.as_deref() {
        Some(reference) => chat_turns::incidents_for_request(&mut scope, reference)
        .await
        .map_err(ApiError::from)?,
        None => Vec::new(),
    };
    let hide = hides_text(ctx, principal, std::slice::from_ref(&found)).await?;
    if hide {
        hide_text(&mut found);
    }
    let sandbox = found.sandbox_name.clone();
    scope.commit().await.map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "chat.view",
            subject_type: Some("sandbox"),
            subject_id: found.sandbox_id,
            detail: &serde_json::json!({
                "sandbox": sandbox,
                "session_id": found.session_id,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    Ok(ChatTurnDetail {
        turn: found,
        incidents,
    })
}

/// Record that an administrator opened a live view.
pub async fn record_live_watch(
    ctx: &ServiceContext,
    principal: &Principal,
    what: &str,
) -> Result<(), ApiError> {
    crate::services::require(principal, Permission::SandboxRead)?;
    let what: String = what.chars().take(50).collect();
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "live.watch",
            subject_type: Some("environment"),
            subject_id: Some(principal.environment_id),
            detail: &serde_json::json!({ "what": what }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// What a principal may see of a live chat event; `None` drops it.
pub async fn redact_live_for(
    ctx: &ServiceContext,
    principal: &Principal,
    event: LiveEvent,
) -> Option<LiveEvent> {
    if principal.role == Role::Admin && !admin_may_read_cached(ctx, principal).await {
        return Some(match event {
            LiveEvent::ChatStarted {
                environment_id,
                sandbox_id,
                request_ref,
                ts,
                model,
                host,
                provider,
                account_kind,
                dlp,
                delivered,
                ..
            } => LiveEvent::ChatStarted {
                environment_id,
                sandbox_id,
                request_ref,
                ts,
                model,
                host,
                provider,
                account_kind,
                dlp,
                delivered,
                user_text: String::new(),
            },
            LiveEvent::ChatDelta {
                environment_id,
                sandbox_id,
                request_ref,
                input_tokens,
                output_tokens,
                done,
                ..
            } => LiveEvent::ChatDelta {
                environment_id,
                sandbox_id,
                request_ref,
                text_so_far: String::new(),
                input_tokens,
                output_tokens,
                done,
            },
            other => other,
        });
    }
    Some(event)
}

/// The admin switch, cached for ten seconds: a stream asks per event.
async fn admin_may_read_cached(ctx: &ServiceContext, principal: &Principal) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<Uuid, (bool, std::time::Instant)>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let key = principal.environment_id;
    if let Ok(guard) = cache.lock() {
        if let Some((value, at)) = guard.get(&key) {
            if at.elapsed() < std::time::Duration::from_secs(10) {
                return *value;
            }
        }
    }
    let value = admin_may_read(ctx, principal).await.unwrap_or(true);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, (value, std::time::Instant::now()));
    }
    value
}

/// Whether the text of these rows has to be hidden from this principal.
async fn hides_text(
    ctx: &ServiceContext,
    principal: &Principal,
    rows: &[ChatTurn],
) -> Result<bool, ApiError> {
    if principal.role != Role::Admin {
        return Ok(false);
    }
    let foreign = rows
        .iter()
        .any(|row| row.owner_user_id.is_some_and(|owner| owner != principal.user_id));
    if !foreign {
        return Ok(false);
    }
    Ok(!admin_may_read_cached(ctx, principal).await)
}

/// The store filter for one query and principal.
fn filter_for<'a>(query: &'a TurnQuery, principal: &Principal) -> TurnFilter<'a> {
    TurnFilter {
        sandbox_id: query.sandbox_id,
        session_id: query.session_id.as_deref(),
        model: query.model.as_deref(),
        provider: query.provider.as_deref(),
        dlp: query.dlp.as_deref(),
        account_kind: query.account_kind.as_deref(),
        owner: query.owner,
        q: query.q.as_deref(),
        since: query.since,
        until: query.until,
        before: query.before,
        only_owner: (principal.role != Role::Admin).then_some(principal.user_id),
        limit: query.limit.unwrap_or(100),
    }
}

/// One row of `chat_turns` as the store writes it.
pub(crate) struct NewTurn<'a> {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub sandbox_id: Option<Uuid>,
    pub owner_user_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub request_ref: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub provider: &'a str,
    pub host: Option<&'a str>,
    pub upstream_ip: Option<&'a str>,
    pub upstream_ip_would: Option<&'a str>,
    pub account_kind: Option<&'a str>,
    pub requested_model: Option<&'a str>,
    pub served_model: Option<&'a str>,
    pub router_provider: Option<&'a str>,
    pub router_model: Option<&'a str>,
    pub user_text: Option<&'a str>,
    pub assistant_text: Option<&'a str>,
    pub tool_calls: &'a serde_json::Value,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub status: Option<i32>,
    pub dlp: Option<&'a str>,
    pub delivered: bool,
    pub route: &'a serde_json::Value,
}

/// Narrow the store's row to what a principal may read.
pub(crate) fn hide_text(turn: &mut ChatTurn) {
    turn.user_text = None;
    turn.assistant_text = None;
    turn.tool_calls = serde_json::json!([]);
}
