//! Telemetry ingest, live publish, and history.
//!
//! Two entry points write telemetry: `ingest` for one event from a gateway or a
//! probe, and `ingest_batch` for the access-log batch a node's egress proxy
//! ships. The batch path is the one that carries bodies, MCP frames, and DNS
//! answers, so it is also the one where the limits live.

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::egress::{channel_of, ChannelFacts, TrafficChannel};
use crate::domain::mcp::{parse_mcp_frame, McpFrame};
use crate::domain::telemetry::{
    Decision, Direction, DnsEvent, EventKind, LiveEvent, NetworkEvent, TelemetryEvent,
};
use crate::error::{ApiError, ErrorCode};
use crate::store::postgres::telemetry as store;
use crate::store::postgres::traffic_bodies;
use crate::store::TenantScope;

/// How many events one batch may carry.
///
/// The proxy batches for latency; it does not get to decide how many rows a
/// single request writes. 500 is a few seconds of a busy node, and a proxy that
/// exceeds it is either misconfigured or trying to flood the table — both worth
/// a refused request rather than a slow insert.
pub const MAX_EVENTS_PER_BATCH: usize = 500;

/// How much decoded body text one batch may carry in total.
///
/// Each side is already capped at 64 KiB by the proxy and by
/// `traffic_bodies::MAX_BODY_BYTES`; this is the second guard, so 500 bodies
/// cannot turn one request into 32 MiB of text.
pub const MAX_BATCH_BODY_BYTES: usize = 4 * 1024 * 1024;

/// One access-log batch from a node's egress proxy.
///
/// The shape is deliberately flat and tolerant: a proxy version that does not
/// know a field yet simply omits it. The environment is named explicitly
/// because a node serves more than one, and every event in the batch belongs to
/// it until a sandbox says otherwise — which the service checks.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyBatch {
    pub environment_id: Uuid,
    #[serde(default)]
    pub traffic: Vec<ProxyTrafficEntry>,
    #[serde(default)]
    pub dns: Vec<ProxyDnsEntry>,
}

/// What is stored in place of a body that could not be masked. A reader sees
/// that something was withheld, and no value is kept.
const WITHHELD_BODY: &str = "[body withheld: masking failed]";

/// One access-log line, as Envoy writes it, plus the captured bodies.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyTrafficEntry {
    pub ts: Option<DateTime<Utc>>,
    pub sandbox_id: Option<Uuid>,
    pub host: String,
    pub port: Option<i32>,
    pub protocol: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<i32>,
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub bytes_in: i64,
    #[serde(default)]
    pub bytes_out: i64,
    /// The sandbox's address as the proxy saw it.
    pub client: Option<String>,
    /// The listener that logged the line: `proxy`, `terminated`, `inbound`,
    /// `vpn`. Absent from a proxy version that does not name it.
    pub listener: Option<String>,
    /// Whether the line is a CONNECT, as the proxy wrote it (`true`, `""`).
    /// A string because Envoy's JSON log writes dynamic metadata as text.
    pub connect: Option<String>,
    /// The name a CONNECT asked for. The only destination a tunnel reveals.
    pub sni: Option<String>,
    /// What the proxy recognised the application protocol as.
    pub app_protocol: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
    /// `allow`, `deny`, or `audit`. Anything else is read as a deny.
    pub decision: Option<String>,
    /// The rules' verdict when the request was let through anyway: `1` or
    /// `true` when the destination would have been denied. Envoy writes it as
    /// text, so it is read as a string and normalised below.
    #[serde(default)]
    pub would_deny: Option<String>,
    pub reason: Option<String>,
    pub rule_id: Option<Uuid>,
    pub mcp_session_id: Option<String>,
    pub mcp_method: Option<String>,
    pub mcp_tool: Option<String>,
    /// The registry key of the MCP server, when the proxy knows it.
    pub mcp_server: Option<String>,
    /// Base64, as Envoy's JSON access-log field carries it.
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    /// Token counters the proxy read off a model answer, as compact JSON
    /// (`i`, `o`, `cr`, `cw`, `cw1`, `rs`). Absent for anything else.
    #[serde(default)]
    pub usage: Option<String>,
    /// How many `cache_control` markers the request carried.
    #[serde(default)]
    pub cache_markers: Option<String>,
    /// `key` or `account`: how the call was authenticated. Never the credential.
    #[serde(default)]
    pub llm_auth: Option<String>,
    /// The address the request reached, as the broker observed it.
    #[serde(default)]
    pub upstream_ip: Option<String>,
    /// The address a refused request would have reached.
    #[serde(default)]
    pub upstream_ip_would: Option<String>,
    /// What Envoy reports as the upstream of a tunnel.
    #[serde(default)]
    pub upstream_host: Option<String>,
    /// Which provider the router actually used, when it says.
    #[serde(default)]
    pub router_provider: Option<String>,
    /// Which model the router actually used, when it says.
    #[serde(default)]
    pub router_model: Option<String>,
    /// What the content check did with this request, as the broker reported it:
    /// `blocked`, `replaced`, `pseudonymized`, `unavailable`, `too_large`. Absent
    /// when the check does not apply (a tunnel, a non-JSON body, an exempt host).
    #[serde(default)]
    pub dlp: Option<String>,
}

/// One DNS answer, from the node's resolver.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyDnsEntry {
    pub ts: Option<DateTime<Utc>>,
    pub sandbox_id: Option<Uuid>,
    pub hostname: String,
    /// `A`, `AAAA`, `CNAME`, `TXT`, … as the resolver recorded it.
    pub qtype: Option<String>,
    pub answer: Option<String>,
    pub ttl: Option<i32>,
    pub decision: Option<String>,
    pub resolver: Option<String>,
    pub request_id: Option<String>,
}

/// What a batch produced, for the proxy's log line.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BatchOutcome {
    /// Events the batch carried.
    pub accepted: usize,
    /// Rows actually written (a retried batch writes fewer).
    pub stored: u64,
    pub bodies: u64,
    pub mcp_calls: u64,
}

/// Accept one event from a gateway or the runtime.
///
/// Two independent effects: persist it for history, and publish it live. A slow
/// database must not delay the stream, so the publish happens whether or not the
/// insert succeeded; a lost history row is a gap in a report, a lost live event
/// is a console that shows nothing while an agent works.
pub async fn ingest(ctx: &ServiceContext, event: TelemetryEvent) -> Result<Uuid, ApiError> {
    let environment_id = event.environment_id();
    let sandbox_id = event.sandbox_id();
    let kind = event.kind();
    let ts = match &event {
        TelemetryEvent::Network(e) => e.ts,
        TelemetryEvent::Dns(e) => e.ts,
        TelemetryEvent::Inference(e) => e.ts,
    };

    let detail = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);

    // A sandbox whose recording is off stores nothing; the live stream still runs.
    let muted = match sandbox_id {
        Some(sandbox_id) => mute_states(ctx, environment_id, &[sandbox_id])
            .await
            .get(&sandbox_id)
            .is_some_and(|state| state.all),
        None => false,
    };

    let id = if muted {
        Uuid::new_v4()
    } else {
        let stored = store::insert(
            &ctx.pool,
            store::NewEvent {
                environment_id,
                sandbox_id,
                gateway_id: None,
                kind,
                source: Some("gateway"),
                ts,
                detail,
            },
        )
        .await;
        match stored {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(error = %error, "telemetry insert failed; publishing anyway");
                Uuid::new_v4()
            }
        }
    };

    if !muted {
        if let TelemetryEvent::Inference(inference) = &event {
            if let Err(error) = store::insert_inference(&ctx.pool, inference).await {
                tracing::warn!(error = %error, "inference row insert failed");
            }
        }
    }

    ctx.bus.publish(LiveEvent::Telemetry(event));
    Ok(id)
}

/// Ingest one batch of access-log lines from a node's egress proxy.
///
/// The order is the security story:
///
/// 1. **Authenticate the node.** A batch names an environment; only a node with
///    a resolvable token may speak for one. Every failure is the same 401, so a
///    probe cannot tell "unknown node" from "wrong token".
/// 2. **Refuse an oversized batch** before a single row is written.
/// 3. **Check the tenant.** The environment must exist and every named sandbox
///    must belong to it, or the batch is refused as a whole. A partial write
///    would leave one tenant's timeline with another tenant's traffic.
/// 4. **Parse, then store, then publish.** Parsing happens first so a malformed
///    body is a 422 rather than a stored half-event. The live publish follows
///    the store attempt, because a console that shows nothing while a node is
///    reporting is the failure this stream exists to prevent.
pub async fn ingest_batch(
    ctx: &ServiceContext,
    node_reference: &str,
    node_token: &str,
    batch: ProxyBatch,
) -> Result<BatchOutcome, ApiError> {
    authenticate_node(ctx, node_reference, node_token).await?;
    check_batch_size(&batch)?;

    if !store::environment_exists(&ctx.pool, batch.environment_id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::forbidden("telemetry.write"));
    }
    ensure_known_sandboxes(ctx, &batch).await?;

    let now = Utc::now();
    let mut prepared = Vec::with_capacity(batch.traffic.len() + batch.dns.len());
    let mut body_bytes = 0usize;

    for entry in &batch.traffic {
        let event = prepare_traffic(entry, batch.environment_id, now)?;
        body_bytes += event.body_bytes();
        prepared.push(event);
    }
    for entry in &batch.dns {
        prepared.push(prepare_dns(entry, batch.environment_id, now)?);
    }

    if body_bytes > MAX_BATCH_BODY_BYTES {
        return Err(ApiError::unprocessable(
            ErrorCode::PAYLOAD_TOO_LARGE,
            format!(
                "A batch carries at most {MAX_BATCH_BODY_BYTES} bytes of body text; \
                 this one carries {body_bytes}."
            ),
        ));
    }

    // Recording mutes (B11), read once per batch. A muted sandbox's events are
    // not stored (`all`) or stored without bodies, chats and tool calls (`bodies`);
    // the live publish below is not affected.
    let sandbox_ids: Vec<Uuid> = prepared.iter().filter_map(|item| item.sandbox_id).collect();
    let mutes = mute_states(ctx, batch.environment_id, &sandbox_ids).await;
    let state_of = |item: &PreparedEvent| -> MuteState {
        item.sandbox_id
            .and_then(|id| mutes.get(&id).copied())
            .unwrap_or_default()
    };

    let events: Vec<store::BatchEvent<'_>> = prepared
        .iter()
        .filter(|item| !state_of(item).all)
        .map(|item| store::BatchEvent {
            id: item.id,
            environment_id: batch.environment_id,
            sandbox_id: item.sandbox_id,
            gateway_id: None,
            kind: item.kind,
            source: Some("egress-proxy"),
            ts: item.ts,
            detail: item.detail.clone(),
        })
        .collect();

    // A captured body is the request as it *arrived*, which is before the content
    // check rewrote it. Storing that would keep the value the policy promised not
    // to keep, so the batch is scrubbed first — always, whatever the mode.
    let mut scrubbed: Vec<Option<(String, String)>> = Vec::with_capacity(prepared.len());
    for item in &prepared {
        let state = state_of(item);
        match &item.body {
            None => scrubbed.push(None),
            Some(_) if state.all || state.bodies => scrubbed.push(None),
            Some(body) => {
                let texts: Vec<String> = [body.request.as_ref(), body.response.as_ref()]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect();
                // A body that cannot be masked is not stored at all. Keeping it
                // as it arrived would hold the value the policy promised never to
                // keep, and an unattributable body is not worth that.
                let masked = match item.sandbox_id {
                    None => texts.iter().map(|_| WITHHELD_BODY.to_string()).collect(),
                    Some(sandbox_id) => {
                        let items: Vec<(Uuid, String)> =
                            texts.iter().map(|text| (sandbox_id, text.clone())).collect();
                        crate::services::privacy_service::scrub_batch(
                            ctx,
                            batch.environment_id,
                            &items,
                        )
                        .await
                        .unwrap_or_else(|error| {
                            tracing::warn!(?error, "cannot mask a captured body; withholding it");
                            texts.iter().map(|_| WITHHELD_BODY.to_string()).collect()
                        })
                    }
                };
                let mut masked = masked.into_iter();
                let request = body.request.as_ref().map(|_| masked.next().unwrap_or_default());
                let response = body.response.as_ref().map(|_| masked.next().unwrap_or_default());
                scrubbed.push(Some((request.unwrap_or_default(), response.unwrap_or_default())));
            }
        }
    }

    let bodies: Vec<traffic_bodies::NewBody<'_>> = prepared
        .iter()
        .zip(scrubbed.iter())
        .filter_map(|(item, masked)| {
            let body = item.body.as_ref()?;
            let (request, response) = masked.as_ref()?;
            Some(traffic_bodies::NewBody {
                event_id: item.id,
                sandbox_id: item.sandbox_id,
                ts: item.ts,
                request_body: body.request.as_ref().map(|_| request.as_str()),
                response_body: body.response.as_ref().map(|_| response.as_str()),
                request_bytes: body.request_bytes,
                response_bytes: body.response_bytes,
                truncated: body.truncated,
            })
        })
        .collect();

    let calls: Vec<store::NewToolCall> = prepared
        .iter()
        .filter(|item| {
            let state = state_of(item);
            !state.all && !state.bodies
        })
        .filter_map(|item| item.call.clone())
        .collect();

    // Each step is attempted even when an earlier one failed, so one broken
    // statement does not also cost the bodies that arrived with it. The first
    // error is reported after the live publish, and the proxy retries the batch;
    // the ids make the retry idempotent.
    let stored = store::insert_batch(&ctx.pool, batch.environment_id, &events).await;
    let body_rows = traffic_bodies::insert_batch(&ctx.pool, batch.environment_id, &bodies).await;
    let mcp_rows = store::insert_tool_calls(&ctx.pool, batch.environment_id, &calls).await;

    // One row per model call: the chat views read this, not the captured bodies.
    // The texts are the masked ones, so a turn can never hold a value.
    let turns: Vec<crate::services::chat_service::TurnInput> = prepared
        .iter()
        .zip(scrubbed.iter())
        .filter_map(|(item, masked)| {
            let sandbox_id = item.sandbox_id?;
            let detail = &item.detail;
            let text = |key: &str| {
                detail
                    .get(key)
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            };
            let (request, response) = masked.as_ref()?;
            Some(crate::services::chat_service::TurnInput {
                id: item.id,
                sandbox_id: Some(sandbox_id),
                ts: item.ts,
                ended_at: None,
                request_ref: text("request_id"),
                method: text("method").unwrap_or_default(),
                path: text("path").unwrap_or_default(),
                host: text("host").unwrap_or_default(),
                user_agent: text("user_agent"),
                upstream_ip: text("upstream_ip"),
                upstream_ip_would: text("upstream_ip_would"),
                account_kind: text("account_kind"),
                router_provider: text("router_provider"),
                router_model: text("router_model"),
                status: detail
                    .get("status")
                    .and_then(|value| value.as_i64())
                    .map(|value| value as i32),
                dlp: text("dlp"),
                channel: text("channel"),
                decision: text("decision"),
                request_body: Some(request.clone()),
                response_body: Some(response.clone()),
            })
        })
        .collect();
    if let Err(error) =
        crate::services::chat_service::record_turns(ctx, batch.environment_id, &turns).await
    {
        tracing::warn!(?error, "cannot record chat turns");
    }

    for item in &prepared {
        ctx.bus.publish(LiveEvent::Telemetry(item.live.clone()));
    }

    let stored = stored.map_err(|error| {
        tracing::error!(error = %error, "telemetry batch insert failed");
        ApiError::from(error)
    })?;
    let body_rows = body_rows.map_err(|error| {
        tracing::error!(error = %error, "traffic body insert failed");
        ApiError::from(error)
    })?;
    let mcp_rows = mcp_rows.map_err(|error| {
        tracing::error!(error = %error, "mcp tool call insert failed");
        ApiError::from(error)
    })?;

    // Model usage the proxy read, into the accounting table. After the traffic is
    // stored and best effort: a failure here is logged and must not turn into a
    // refused batch (the proxy would retry it and the traffic is already in).
    for (entry, item) in batch.traffic.iter().zip(prepared.iter()) {
        if let Some(call) = observed_call(entry, item.id, batch.environment_id) {
            if let Err(error) = crate::services::accounting_service::record_observed(ctx, call).await {
                tracing::warn!(code = error.code.0, "could not record observed model usage");
            }
        }
    }

    Ok(BatchOutcome {
        accepted: prepared.len(),
        stored,
        bodies: body_rows,
        mcp_calls: mcp_rows,
    })
}

/// Refuse a batch that is empty or too large.
///
/// Separate from the handler so the limit is a rule with a test, not a constant
/// someone has to remember to apply.
pub fn check_batch_size(batch: &ProxyBatch) -> Result<(), ApiError> {
    let total = batch.traffic.len() + batch.dns.len();
    if total == 0 {
        return Err(ApiError::validation("A batch carries no events."));
    }
    if total > MAX_EVENTS_PER_BATCH {
        return Err(ApiError::unprocessable(
            ErrorCode::PAYLOAD_TOO_LARGE,
            format!(
                "A batch carries at most {MAX_EVENTS_PER_BATCH} events; this one carries {total}."
            ),
        ));
    }
    Ok(())
}

/// Verify the node's token against the file its row points at.
///
/// Fail-closed at every branch: no header, no row, no locator, no token file, or
/// a token file that does not match are all the same refusal. The comparison is
/// over SHA-256 hashes with a constant-time equality, so a token cannot be
/// guessed one byte at a time from response timing.
async fn authenticate_node(
    ctx: &ServiceContext,
    node_reference: &str,
    presented_token: &str,
) -> Result<Uuid, ApiError> {
    let reference = node_reference.trim();
    let presented = presented_token.trim();
    let refused = || ApiError::unauthenticated("This endpoint requires a valid node token.");

    if reference.is_empty() || presented.is_empty() {
        tracing::warn!("telemetry batch refused: no node reference or no token");
        return Err(refused());
    }

    let node = store::node_identity(&ctx.pool, reference)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            tracing::warn!(node = %reference, "telemetry batch refused: unknown node");
            refused()
        })?;

    let locator = node.token_ref.as_deref().ok_or_else(|| {
        tracing::warn!(node = %node.name, "telemetry batch refused: node has no token configured");
        refused()
    })?;

    // One rule for every node call: file locators (the appliance) and digest locators (a node
    // that joined over the overlay) are verified in the same place, in constant time.
    if let Err(error) = crate::security::node_token::verify(locator, &ctx.secret_dir, presented) {
        tracing::warn!(node = %node.name, reason = ?error, "telemetry batch refused: node token");
        return Err(refused());
    }

    Ok(node.id)
}

/// Every sandbox the batch names must exist and belong to the batch's tenant.
///
/// The check runs in one tenant scope for all ids, so a batch with a thousand
/// entries is still one query. A batch that fails it writes nothing.
async fn ensure_known_sandboxes(
    ctx: &ServiceContext,
    batch: &ProxyBatch,
) -> Result<(), ApiError> {
    let mut declared: Vec<Uuid> = batch
        .traffic
        .iter()
        .filter_map(|entry| entry.sandbox_id)
        .chain(batch.dns.iter().filter_map(|entry| entry.sandbox_id))
        .collect();
    declared.sort_unstable();
    declared.dedup();

    if declared.is_empty() {
        return Ok(());
    }

    let mut scope = TenantScope::begin(&ctx.pool, batch.environment_id)
        .await
        .map_err(ApiError::from)?;
    let found = store::sandboxes_in_environment(&mut scope, &declared)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    if found.len() != declared.len() {
        tracing::warn!(
            environment = %batch.environment_id,
            declared = declared.len(),
            found = found.len(),
            "telemetry batch refused: a sandbox is not in the environment"
        );
        return Err(ApiError::forbidden("telemetry.write"));
    }
    Ok(())
}

/// What a sandbox's recording mutes say right now.
#[derive(Debug, Clone, Copy, Default)]
struct MuteState {
    /// Nothing is stored (`telemetry_mute_until` is in the future).
    all: bool,
    /// Flows are stored, bodies, chats and tool calls are not.
    bodies: bool,
}

/// Read the recording mutes of some sandboxes, once, for one call (B11).
///
/// A read that fails is logged and treated as "not muted": the same database
/// error would stop the insert as well, so nothing is stored by mistake.
async fn mute_states(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_ids: &[Uuid],
) -> std::collections::HashMap<Uuid, MuteState> {
    let mut out = std::collections::HashMap::new();
    let mut unique: Vec<Uuid> = sandbox_ids.to_vec();
    unique.sort();
    unique.dedup();
    if unique.is_empty() {
        return out;
    }
    let now = Utc::now();
    let mut scope = match TenantScope::begin(&ctx.pool, environment_id).await {
        Ok(scope) => scope,
        Err(error) => {
            tracing::warn!(?error, "cannot read recording mutes");
            return out;
        }
    };
    for id in unique {
        match crate::store::postgres::sandboxes::telemetry_mute(&mut scope, id).await {
            Ok((all, bodies)) => {
                out.insert(
                    id,
                    MuteState {
                        all: all.is_some_and(|until| until > now),
                        bodies: bodies.is_some_and(|until| until > now),
                    },
                );
            }
            Err(error) => tracing::warn!(?error, "cannot read recording mutes"),
        }
    }
    if let Err(error) = scope.commit().await {
        tracing::warn!(?error, "cannot close the recording mute read");
    }
    out
}

/// One event, built and ready to store.
struct PreparedEvent {
    id: Uuid,
    ts: DateTime<Utc>,
    sandbox_id: Option<Uuid>,
    kind: EventKind,
    detail: serde_json::Value,
    body: Option<PreparedBody>,
    call: Option<store::NewToolCall>,
    live: TelemetryEvent,
}

impl PreparedEvent {
    /// Decoded body text this event carries, for the batch cap.
    fn body_bytes(&self) -> usize {
        match &self.body {
            Some(body) => {
                body.request.as_ref().map_or(0, String::len)
                    + body.response.as_ref().map_or(0, String::len)
            }
            None => 0,
        }
    }
}

/// One event's bodies, already decoded and cut.
struct PreparedBody {
    request: Option<String>,
    response: Option<String>,
    request_bytes: i64,
    response_bytes: i64,
    truncated: bool,
}

/// A decoded body, with what was actually in it before the cap.
struct DecodedBody {
    text: String,
    bytes: i64,
    truncated: bool,
}

/// Build the network event, its body row, and its MCP call from one log line.
fn prepare_traffic(
    entry: &ProxyTrafficEntry,
    environment_id: Uuid,
    now: DateTime<Utc>,
) -> Result<PreparedEvent, ApiError> {
    let host = entry.host.trim();
    if host.is_empty() {
        return Err(ApiError::validation("A traffic entry names no host."));
    }

    let request = decode_body(entry.request_body.as_deref())?;
    let response = decode_body(entry.response_body.as_deref())?;

    // Recognition comes from the body itself, not only from what the proxy
    // labelled: a proxy version without the Lua filter still logs the call.
    let frame = request
        .as_ref()
        .and_then(|body| parse_mcp_frame(body.text.as_bytes()));
    let labelled_mcp = entry.mcp_method.is_some()
        || entry.mcp_tool.is_some()
        || entry.mcp_session_id.is_some();
    let is_mcp = frame.is_some() || labelled_mcp;

    let id = event_id(environment_id, entry.sandbox_id, entry.request_id.as_deref(), entry.ts, now);
    let ts = entry.ts.unwrap_or(now);
    let decision = parse_decision(entry.decision.as_deref());

    // The channel is decided from what the proxy observed, once, here: the read
    // path then colours a row without re-deriving it from three fields.
    let channel = channel_of(&ChannelFacts {
        listener: entry.listener.as_deref(),
        connect: truthy(entry.connect.as_deref()),
        sni: entry.sni.as_deref(),
        client: entry.client.as_deref(),
    });
    let direction = if channel == TrafficChannel::Inbound {
        Direction::Inbound
    } else {
        Direction::Outbound
    };

    let event = NetworkEvent {
        id,
        environment_id,
        sandbox_id: entry.sandbox_id,
        gateway_id: None,
        ts,
        direction,
        protocol: wire_protocol(entry, is_mcp),
        source: entry
            .client
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("proxy")
            .to_string(),
        destination: host.to_string(),
        port: entry.port,
        binary: None,
        decision,
        rule_id: entry.rule_id,
        rule_name: None,
        latency_ms: entry
            .duration_ms
            .map(|ms| ms.clamp(0, i64::from(i32::MAX)) as i32),
        correlation_id: entry.request_id.clone(),
        enforcement: Some(
            match decision {
                Decision::Audit => "audit",
                _ => "enforced",
            }
            .to_string(),
        ),
        channel: Some(channel.as_str().to_string()),
        sni: entry
            .sni
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        listener: entry
            .listener
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        app_protocol: entry
            .app_protocol
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    };

    let live = TelemetryEvent::Network(event);
    let mut detail = serde_json::to_value(&live).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut detail {
        // Facts the domain event does not carry but the traffic table shows.
        // Additive keys keep the event shape intact for the SSE consumer.
        if let Some(status) = entry.status {
            map.insert("status".into(), serde_json::json!(status));
        }
        map.insert("bytes_in".into(), serde_json::json!(entry.bytes_in));
        map.insert("bytes_out".into(), serde_json::json!(entry.bytes_out));
        map.insert(
            "has_body".into(),
            serde_json::json!(request.is_some() || response.is_some()),
        );
        map.insert(
            "inspected".into(),
            serde_json::json!(channel == TrafficChannel::Inspected || entry.path.is_some()),
        );
        // The rules still ran when a window or an "allow everything" mode let
        // the request through. Recording their verdict is what makes a target
        // that "would have been blocked" visible in the decisions view.
        map.insert(
            "would_deny".into(),
            serde_json::json!(would_deny_flag(entry.would_deny.as_deref())),
        );
        if let Some(method) = entry.method.as_deref().filter(|value| !value.is_empty()) {
            map.insert("method".into(), serde_json::json!(method));
        }
        if let Some(path) = entry.path.as_deref().filter(|value| !value.is_empty()) {
            map.insert("path".into(), serde_json::json!(path));
        }
        if let Some(reason) = entry.reason.as_deref().filter(|value| !value.is_empty()) {
            map.insert("reason".into(), serde_json::json!(reason));
        }
        if let Some(tool) = entry.mcp_tool.as_deref().filter(|value| !value.is_empty()) {
            map.insert("mcp_tool".into(), serde_json::json!(tool));
        }
        // A refused request is a fact about the chat, not only about the proxy:
        // the console shows "not delivered" from this field.
        if let Some(dlp) = entry.dlp.as_deref().filter(|value| !value.is_empty()) {
            map.insert("dlp".into(), serde_json::json!(dlp));
        }
        // The address the request actually reached, and the one a refusal would
        // have reached: "it went to Anthropic" is not the same fact as "it would
        // have gone to 160.79.104.10".
        for (key, value) in [
            ("upstream_ip", entry.upstream_ip.as_deref()),
            ("upstream_ip_would", entry.upstream_ip_would.as_deref()),
            ("upstream_host", entry.upstream_host.as_deref()),
            ("router_provider", entry.router_provider.as_deref()),
            ("router_model", entry.router_model.as_deref()),
            ("account_kind", entry.llm_auth.as_deref()),
        ] {
            if let Some(value) = value.filter(|value| !value.is_empty() && *value != "-") {
                map.insert(key.into(), serde_json::json!(value));
            }
        }
    }

    let call = if is_mcp {
        build_mcp_call(entry, id, ts, frame.as_ref(), response.as_ref())
    } else {
        None
    };

    let body = match (&request, &response) {
        (None, None) => None,
        _ => Some(PreparedBody {
            request: request.as_ref().map(|body| body.text.clone()),
            response: response.as_ref().map(|body| body.text.clone()),
            request_bytes: request.as_ref().map_or(0, |body| body.bytes),
            response_bytes: response.as_ref().map_or(0, |body| body.bytes),
            truncated: request.as_ref().is_some_and(|body| body.truncated)
                || response.as_ref().is_some_and(|body| body.truncated),
        }),
    };

    Ok(PreparedEvent {
        id,
        ts,
        sandbox_id: entry.sandbox_id,
        kind: EventKind::Network,
        detail,
        body,
        call,
        live,
    })
}

/// Build the DNS event, with the record facts kept in the stored detail.
///
/// `DnsEvent` answers "which name did the sandbox ask for, and was it allowed".
/// The record type, answer, and TTL are what an operator needs next, and they
/// are additions to the stored detail rather than new domain fields, so the SSE
/// consumer keeps one shape.
fn prepare_dns(
    entry: &ProxyDnsEntry,
    environment_id: Uuid,
    now: DateTime<Utc>,
) -> Result<PreparedEvent, ApiError> {
    let hostname = entry.hostname.trim();
    if hostname.is_empty() {
        return Err(ApiError::validation("A DNS entry names no hostname."));
    }
    let hostname = hostname.to_string();
    let ts = entry.ts.unwrap_or(now);
    let id = event_id(environment_id, entry.sandbox_id, entry.request_id.as_deref(), entry.ts, now);
    let decision = parse_decision(entry.decision.as_deref());

    let event = DnsEvent {
        id,
        environment_id,
        sandbox_id: entry.sandbox_id,
        ts,
        hostname,
        resolver: entry
            .resolver
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        decision,
        binary: None,
        correlation_id: entry.request_id.clone(),
    };

    let live = TelemetryEvent::Dns(event);
    let mut detail = serde_json::to_value(&live).unwrap_or(serde_json::Value::Null);
    if let serde_json::Value::Object(map) = &mut detail {
        if let Some(qtype) = entry.qtype.as_deref().filter(|value| !value.trim().is_empty()) {
            map.insert("qtype".into(), serde_json::json!(qtype));
        }
        if let Some(answer) = entry.answer.as_deref().filter(|value| !value.trim().is_empty()) {
            map.insert("answer".into(), serde_json::json!(answer));
        }
        if let Some(ttl) = entry.ttl {
            map.insert("ttl".into(), serde_json::json!(ttl));
        }
    }

    Ok(PreparedEvent {
        id,
        ts,
        sandbox_id: entry.sandbox_id,
        kind: EventKind::Dns,
        detail,
        body: None,
        call: None,
        live,
    })
}

/// Build the `mcp_tool_calls` row from a recognised frame and its response.
///
/// Returns `None` when the call has no method at all: a row that says "something
/// happened" is worse than no row, because the MCP view would show a nameless
/// call and an operator would have to guess.
fn build_mcp_call(
    entry: &ProxyTrafficEntry,
    event_id: Uuid,
    ts: DateTime<Utc>,
    frame: Option<&McpFrame>,
    response: Option<&DecodedBody>,
) -> Option<store::NewToolCall> {
    let method = frame
        .and_then(|frame| frame.method.clone())
        .or_else(|| entry.mcp_method.clone())?;

    let response_value = response.map(|body| {
        serde_json::from_str::<serde_json::Value>(&body.text)
            .unwrap_or_else(|_| serde_json::Value::String(body.text.clone()))
    });

    let result = response_value.map(|value| {
        let is_error = json_rpc_is_error(&value) || entry.status.is_some_and(|code| code >= 400);
        (value, is_error)
    });
    let is_error = result
        .as_ref()
        .map(|(_, is_error)| *is_error)
        .unwrap_or_else(|| entry.status.is_some_and(|code| code >= 400));

    let call_id = deterministic_id(&format!("nestbox-mcp:{event_id}:{method}"));

    Some(store::NewToolCall {
        id: call_id,
        sandbox_id: entry.sandbox_id,
        ts,
        server_host: entry.host.trim().to_string(),
        server_key: entry
            .mcp_server
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        session_id: entry.mcp_session_id.clone(),
        method,
        tool_name: frame
            .and_then(|frame| frame.tool_name.clone())
            .or_else(|| entry.mcp_tool.clone()),
        arguments: frame.and_then(|frame| frame.arguments.clone()),
        result: result.map(|(value, _)| value),
        is_error,
        duration_ms: entry
            .duration_ms
            .map(|ms| ms.clamp(0, i64::from(i32::MAX)) as i32),
        request_id: entry.request_id.clone(),
        event_id: Some(event_id),
    })
}

/// Whether a JSON-RPC response is an error, in either supported form.
///
/// Two forms exist and both matter: a transport-level `error` object, and the
/// MCP tool result flag `isError`. Looking only at the HTTP status would report
/// a failed tool call as a success, because MCP returns tool failures with 200.
fn json_rpc_is_error(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => {
            map.contains_key("error")
                || map
                    .get("result")
                    .and_then(|result| result.get("isError"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        }
        serde_json::Value::Array(items) => items.iter().any(json_rpc_is_error),
        _ => false,
    }
}

/// The id of one event.
///
/// With a request id the id is derived, so a batch the proxy retries after a
/// timeout does not double the timeline; without one there is nothing to derive
/// from and a random id is the honest choice.
fn event_id(
    environment_id: Uuid,
    sandbox_id: Option<Uuid>,
    request_id: Option<&str>,
    ts: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Uuid {
    let request_id = request_id.map(str::trim).filter(|value| !value.is_empty());
    match request_id {
        Some(request_id) => {
            let stamp = ts.unwrap_or(now).timestamp_millis();
            let sandbox = sandbox_id.map(|id| id.to_string()).unwrap_or_default();
            deterministic_id(&format!(
                "nestbox-egress:{environment_id}:{sandbox}:{request_id}:{stamp}"
            ))
        }
        None => Uuid::new_v4(),
    }
}

/// A UUID derived from a key, so a retried batch keeps its ids.
///
/// SHA-256 rather than RFC 4122 version 5 (which is SHA-1) because SHA-256 is
/// already a dependency and the only property needed is "same input, same id".
/// The version and variant bits are set anyway, so a reader can tell a derived
/// id from a random one.
fn deterministic_id(key: &str) -> Uuid {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

/// What the proxy decided, read conservatively.
///
/// An absent or unrecognised decision is a deny. The alternative — defaulting to
/// allow — would let a proxy version that stopped sending the field turn every
/// blocked request into a green row.
/// Whether a proxy-written boolean is true.
///
/// Envoy's JSON access log writes dynamic metadata as text, so a boolean arrives
/// as `"true"`, `"1"`, or an empty string rather than as a JSON `true`. Anything
/// unrecognised is false: a field that is not clearly set is not a claim.
fn truthy(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("true") | Some("1") | Some("yes") | Some("on")
    )
}

/// Whether the rules would have denied a request the proxy let through.
///
/// The egress proxy's access log carries the ext_authz answer's `would_deny`
/// as text: `1` when the rules said deny, `0` when they said allow, and a dash
/// when the filter did not run at all. Only an explicit yes counts, so a line
/// from a proxy version without the field is not turned into a claim.
fn would_deny_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true")
    )
}

fn parse_decision(raw: Option<&str>) -> Decision {
    raw.map(str::trim)
        .and_then(Decision::parse)
        .unwrap_or(Decision::Deny)
}

/// The protocol as the console groups it.
///
/// Envoy logs `%PROTOCOL%` as `HTTP/1.1` or `HTTP/2`; the console filters on
/// `http`, `https`, `tcp`, `mcp`, and `dns`. Anything else is passed through
/// lowercased rather than flattened, so a new protocol is visible as itself
/// instead of looking like a mistake.
fn wire_protocol(entry: &ProxyTrafficEntry, is_mcp: bool) -> String {
    if is_mcp {
        return "mcp".to_string();
    }
    match entry.protocol.as_deref() {
        Some(raw) => {
            let lowered = raw.trim().to_ascii_lowercase();
            if lowered.starts_with("http") {
                if entry.port == Some(443) {
                    "https".to_string()
                } else {
                    "http".to_string()
                }
            } else if lowered.is_empty() {
                "tcp".to_string()
            } else {
                lowered
            }
        }
        None => "tcp".to_string(),
    }
}

/// Decode one body and enforce the per-side cap.
///
/// The proxy is expected to cap at 64 KiB; this repeats the cap because the
/// proxy is a client, not an authority. A longer body is cut and flagged, not
/// refused: losing one body should not also lose the event that points at it.
fn decode_body(raw: Option<&str>) -> Result<Option<DecodedBody>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }

    let engine = base64::engine::general_purpose::STANDARD;
    let decoded = engine.decode(raw).or_else(|_| {
        base64::engine::general_purpose::STANDARD_NO_PAD.decode(raw)
    });
    let decoded = decoded.map_err(|_| {
        ApiError::validation("A body in the batch is not valid base64.")
    })?;

    let bytes = decoded.len();
    let truncated = bytes > traffic_bodies::MAX_BODY_BYTES;
    let slice = if truncated {
        &decoded[..traffic_bodies::MAX_BODY_BYTES]
    } else {
        &decoded[..]
    };

    Ok(Some(DecodedBody {
        // A body is not guaranteed to be UTF-8: the proxy captures bytes. Lossy
        // conversion keeps the readable part instead of dropping the row.
        text: String::from_utf8_lossy(slice).into_owned(),
        bytes: bytes as i64,
        truncated,
    }))
}

/// Recent events, for the console's history view.
///
/// The time range defaults to one hour: a query without a bound scans every
/// partition, which defeats the partitioning.
pub async fn history(
    ctx: &ServiceContext,
    principal: &Principal,
    kind: Option<EventKind>,
    minutes: i64,
    limit: i64,
) -> Result<Vec<store::StoredEvent>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let since = Utc::now() - Duration::minutes(minutes.clamp(1, 10_080));
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let events = store::list(&mut scope, kind, since, limit.clamp(1, 500))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(events)
}

/// Counts per kind for the filter bar.
pub async fn facets(
    ctx: &ServiceContext,
    principal: &Principal,
    minutes: i64,
) -> Result<std::collections::BTreeMap<String, i64>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let since = Utc::now() - Duration::minutes(minutes.clamp(1, 10_080));
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let facets = store::facets(&mut scope, since).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(facets)
}

/// Inference requests for one sandbox.
pub async fn sandbox_inference(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    limit: i64,
) -> Result<Vec<serde_json::Value>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = store::inference_for_sandbox(&mut scope, sandbox_id, limit.clamp(1, 500))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// Create the partitions the retention policy will later drop.
pub async fn maintain_partitions(
    ctx: &ServiceContext,
    months_ahead: i32,
) -> Result<serde_json::Value, ApiError> {
    let created = store::ensure_partitions(&ctx.pool, months_ahead.clamp(1, 12))
        .await
        .map_err(ApiError::from)?;
    Ok(serde_json::json!({ "created": created }))
}

/// A synthetic event, used by the console's demo mode and by the smoke test.
///
/// Kept in the service rather than the store so it goes through the same redact
/// and publish path as a real event.
pub fn sample_event(environment_id: Uuid, sandbox_id: Option<Uuid>, index: u64) -> TelemetryEvent {
    let destinations = [
        ("api.anthropic.com", 443, "rest", Decision::Allow),
        ("telemetry.example.net", 443, "rest", Decision::Deny),
        ("pypi.org", 443, "rest", Decision::Allow),
        ("169.254.169.254", 80, "http", Decision::Deny),
    ];
    let (destination, port, protocol, decision) =
        destinations[(index as usize) % destinations.len()];

    TelemetryEvent::Network(crate::domain::telemetry::NetworkEvent {
        id: Uuid::new_v4(),
        environment_id,
        sandbox_id,
        gateway_id: None,
        ts: Utc::now(),
        direction: crate::domain::telemetry::Direction::Outbound,
        protocol: protocol.to_string(),
        source: "10.244.0.2".to_string(),
        destination: destination.to_string(),
        port: Some(port),
        binary: Some("/usr/local/bin/agent".to_string()),
        decision,
        rule_id: None,
        rule_name: Some("demo".to_string()),
        latency_ms: Some(12 + (index % 40) as i32),
        correlation_id: Some(format!("demo-{index}")),
        enforcement: Some("enforced".to_string()),
        channel: Some("proxy".to_string()),
        sni: None,
        listener: Some("proxy".to_string()),
        app_protocol: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traffic_entry() -> ProxyTrafficEntry {
        ProxyTrafficEntry {
            ts: None,
            sandbox_id: None,
            dlp: None,
            upstream_ip: None,
            upstream_ip_would: None,
            upstream_host: None,
            router_provider: None,
            router_model: None,
            host: "mcp.example.com".into(),
            port: Some(443),
            protocol: Some("HTTP/2".into()),
            method: Some("POST".into()),
            path: Some("/mcp".into()),
            status: Some(200),
            duration_ms: Some(37),
            bytes_in: 12,
            bytes_out: 34,
            client: Some("10.244.0.2".into()),
            listener: Some("proxy".into()),
            connect: None,
            sni: None,
            app_protocol: None,
            user_agent: Some("hermes/0.22".into()),
            request_id: Some("req-1".into()),
            decision: Some("allow".into()),
            would_deny: None,
            reason: Some("rule".into()),
            rule_id: None,
            mcp_session_id: Some("sess-1".into()),
            mcp_method: None,
            mcp_tool: None,
            mcp_server: Some("nestbox-hub".into()),
            request_body: None,
            response_body: None,
            usage: None,
            cache_markers: None,
            llm_auth: None,
        }
    }

    fn batch_with(traffic: Vec<ProxyTrafficEntry>, dns: Vec<ProxyDnsEntry>) -> ProxyBatch {
        ProxyBatch {
            environment_id: Uuid::nil(),
            traffic,
            dns,
        }
    }

    #[test]
    fn a_sample_event_alternates_allow_and_deny() {
        let env = Uuid::new_v4();
        let mut decisions = Vec::new();
        for index in 0..4 {
            let TelemetryEvent::Network(event) = sample_event(env, None, index) else {
                panic!("expected network");
            };
            decisions.push(event.decision);
        }
        assert!(decisions.contains(&Decision::Allow));
        assert!(decisions.contains(&Decision::Deny));
    }

    #[test]
    fn a_batch_may_not_exceed_its_event_cap() {
        let entry = traffic_entry();
        let at_limit = batch_with(vec![entry.clone(); MAX_EVENTS_PER_BATCH], vec![]);
        assert!(check_batch_size(&at_limit).is_ok());

        let over_limit = batch_with(vec![entry; MAX_EVENTS_PER_BATCH + 1], vec![]);
        let error = check_batch_size(&over_limit).expect_err("refused");
        assert_eq!(error.code, ErrorCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn an_empty_batch_is_refused() {
        let error = check_batch_size(&batch_with(vec![], vec![])).expect_err("refused");
        assert_eq!(error.code, ErrorCode::VALIDATION_FAILED);
    }

    #[test]
    fn a_traffic_entry_becomes_a_network_event() {
        let prepared = prepare_traffic(&traffic_entry(), Uuid::nil(), Utc::now()).expect("built");
        assert_eq!(prepared.kind, EventKind::Network);
        assert_eq!(prepared.detail["kind"], "network");
        assert_eq!(prepared.detail["destination"], "mcp.example.com");
        assert_eq!(prepared.detail["port"], 443);
        assert_eq!(prepared.detail["decision"], "allow");
        assert_eq!(prepared.detail["status"], 200);
        assert_eq!(prepared.detail["bytes_out"], 34);
        assert_eq!(prepared.detail["has_body"], false);
    }

    #[test]
    fn a_missing_decision_is_read_as_a_deny() {
        // Defaulting to allow would turn a proxy that stopped sending the field
        // into a green traffic log.
        assert_eq!(parse_decision(None), Decision::Deny);
        assert_eq!(parse_decision(Some("nonsense")), Decision::Deny);
        assert_eq!(parse_decision(Some("allow")), Decision::Allow);
        assert_eq!(parse_decision(Some("audit")), Decision::Audit);
    }

    #[test]
    fn only_an_explicit_would_deny_is_a_would_deny() {
        // The proxy writes text: "1" for a rule verdict of deny, "0" for allow,
        // "-" when the authz filter never ran. A dash is not a claim.
        assert!(would_deny_flag(Some("1")));
        assert!(would_deny_flag(Some("true")));
        assert!(would_deny_flag(Some(" TRUE ")));
        assert!(!would_deny_flag(Some("0")));
        assert!(!would_deny_flag(Some("-")));
        assert!(!would_deny_flag(Some("")));
        assert!(!would_deny_flag(None));
    }

    #[test]
    fn a_would_deny_line_is_stored_as_a_boolean_in_the_detail() {
        let mut entry = traffic_entry();
        entry.decision = Some("allow".into());
        entry.would_deny = Some("1".into());
        let prepared = prepare_traffic(&entry, Uuid::nil(), Utc::now()).expect("built");
        assert_eq!(prepared.detail["would_deny"], true);

        entry.would_deny = None;
        let prepared = prepare_traffic(&entry, Uuid::nil(), Utc::now()).expect("built");
        assert_eq!(prepared.detail["would_deny"], false);
    }

    #[test]
    fn the_protocol_is_grouped_the_way_the_console_filters_it() {
        let entry = traffic_entry();
        assert_eq!(wire_protocol(&entry, false), "https");
        assert_eq!(wire_protocol(&entry, true), "mcp");

        let mut plain = entry.clone();
        plain.port = Some(80);
        assert_eq!(wire_protocol(&plain, false), "http");

        let mut missing = entry;
        missing.protocol = None;
        assert_eq!(wire_protocol(&missing, false), "tcp");
    }

    #[test]
    fn a_base64_body_is_decoded_and_a_broken_one_is_refused() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"{\"a\":1}");
        let body = decode_body(Some(&encoded)).expect("decoded").expect("body");
        assert_eq!(body.text, "{\"a\":1}");
        assert_eq!(body.bytes, 7);
        assert!(!body.truncated);
        assert!(decode_body(Some("not base64!!")).is_err());
        assert!(decode_body(None).expect("none").is_none());
    }

    #[test]
    fn a_body_beyond_the_cap_is_cut_and_flagged() {
        let raw = vec![b'a'; traffic_bodies::MAX_BODY_BYTES + 10];
        let encoded = base64::engine::general_purpose::STANDARD.encode(&raw);
        let body = decode_body(Some(&encoded)).expect("decoded").expect("body");
        assert!(body.truncated);
        assert_eq!(body.text.len(), traffic_bodies::MAX_BODY_BYTES);
        assert_eq!(body.bytes as usize, raw.len());
    }

    #[test]
    fn an_mcp_post_fills_the_tool_call_from_the_body_and_the_response() {
        let request = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/etc/hostname"}}}"#;
        let response = br#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"host"}],"isError":false}}"#;
        let mut entry = traffic_entry();
        entry.request_body =
            Some(base64::engine::general_purpose::STANDARD.encode(request));
        entry.response_body =
            Some(base64::engine::general_purpose::STANDARD.encode(response));

        let prepared = prepare_traffic(&entry, Uuid::nil(), Utc::now()).expect("built");
        let call = prepared.call.expect("an mcp call");
        assert_eq!(call.method, "tools/call");
        assert_eq!(call.tool_name.as_deref(), Some("read_file"));
        assert_eq!(call.session_id.as_deref(), Some("sess-1"));
        assert_eq!(call.server_key.as_deref(), Some("nestbox-hub"));
        assert_eq!(call.duration_ms, Some(37));
        assert!(!call.is_error);
        assert_eq!(
            call.arguments
                .as_ref()
                .and_then(|value| value.get("path").cloned()),
            Some(serde_json::json!("/etc/hostname"))
        );
    }

    #[test]
    fn an_mcp_error_result_is_recorded_as_an_error() {
        let request = br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"run_command","arguments":{"cmd":"false"}}}"#;
        // MCP reports a failed tool with HTTP 200 and `isError`: the status
        // alone would call this a success.
        let response = br#"{"jsonrpc":"2.0","id":7,"result":{"content":[],"isError":true}}"#;
        let mut entry = traffic_entry();
        entry.request_body =
            Some(base64::engine::general_purpose::STANDARD.encode(request));
        entry.response_body =
            Some(base64::engine::general_purpose::STANDARD.encode(response));

        let prepared = prepare_traffic(&entry, Uuid::nil(), Utc::now()).expect("built");
        let call = prepared.call.expect("an mcp call");
        assert!(call.is_error);
    }

    #[test]
    fn a_json_rpc_error_body_is_an_error_too() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"nope"}}"#,
        )
        .expect("json");
        assert!(json_rpc_is_error(&value));
    }

    #[test]
    fn a_retried_batch_keeps_the_event_id() {
        let ts = Utc::now();
        let first = event_id(Uuid::nil(), None, Some("req-1"), Some(ts), Utc::now());
        let second = event_id(Uuid::nil(), None, Some("req-1"), Some(ts), Utc::now());
        assert_eq!(first, second);

        // Without a request id there is nothing to derive from.
        let a = event_id(Uuid::nil(), None, None, Some(ts), Utc::now());
        let b = event_id(Uuid::nil(), None, None, Some(ts), Utc::now());
        assert_ne!(a, b);
    }

    #[test]
    fn a_dns_entry_becomes_a_dns_event_with_its_record_facts() {
        let entry = ProxyDnsEntry {
            ts: None,
            sandbox_id: None,
            hostname: "api.example.com".into(),
            qtype: Some("A".into()),
            answer: Some("93.184.216.34".into()),
            ttl: Some(60),
            decision: Some("allow".into()),
            resolver: Some("coredns".into()),
            request_id: Some("dns-1".into()),
        };
        let prepared = prepare_dns(&entry, Uuid::nil(), Utc::now()).expect("built");
        assert_eq!(prepared.kind, EventKind::Dns);
        assert_eq!(prepared.detail["kind"], "dns");
        assert_eq!(prepared.detail["hostname"], "api.example.com");
        assert_eq!(prepared.detail["qtype"], "A");
        assert_eq!(prepared.detail["answer"], "93.184.216.34");
        assert_eq!(prepared.detail["ttl"], 60);
        assert!(prepared.body.is_none());

        let nameless = ProxyDnsEntry {
            hostname: "   ".into(),
            ..entry
        };
        assert!(prepare_dns(&nameless, Uuid::nil(), Utc::now()).is_err());
    }

    #[test]
    fn a_plain_json_post_is_not_an_mcp_call() {
        let mut entry = traffic_entry();
        entry.request_body = Some(
            base64::engine::general_purpose::STANDARD
                .encode(br#"{"model":"x","messages":[]}"#),
        );
        let prepared = prepare_traffic(&entry, Uuid::nil(), Utc::now()).expect("built");
        assert!(prepared.call.is_none());
    }
}

/// Take the connection decisions OpenShell supervisors logged into the traffic
/// history, with the sandbox they belong to.
///
/// The runner reads the supervisors' logs; this side owns the sandbox ids, the
/// redaction and the storage, exactly as for the proxy's access log. Event ids are
/// derived from (sandbox, time, destination, verdict), so reading the same log
/// window twice stores each decision once.
pub async fn pull_openshell_events(ctx: &ServiceContext) -> Result<u64, ApiError> {
    let raw = ctx
        .runtime
        .openshell_events(90)
        .await
        .map_err(|error| ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, error.to_string()))?;
    if raw.is_empty() {
        return Ok(0);
    }
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
    let mut stored_total = 0;
    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM sandboxes")
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
        scope.commit().await.map_err(ApiError::from)?;
        let ids: std::collections::HashMap<String, Uuid> = rows.into_iter().map(|(id, name)| (name, id)).collect();

        let now = Utc::now();
        let mut prepared = Vec::new();
        for event in &raw {
            let Some(sandbox_id) = event.get("sandbox").and_then(|v| v.as_str()).and_then(|name| ids.get(name)) else {
                continue;
            };
            let host = event.get("host").and_then(|v| v.as_str()).unwrap_or_default();
            if host.is_empty() {
                continue;
            }
            let seconds = event.get("ts").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ts = DateTime::<Utc>::from_timestamp_millis((seconds * 1000.0) as i64);
            let port = event.get("port").and_then(|v| v.as_i64()).map(|p| p as i32);
            let decision = event.get("decision").and_then(|v| v.as_str()).unwrap_or("deny");
            let dns = event.get("kind").and_then(|v| v.as_str()) == Some("dns");
            let reason = [
                event.get("policy").and_then(|v| v.as_str()).map(|p| format!("policy {p}")),
                event.get("reason").and_then(|v| v.as_str()).map(str::to_string),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("; ");
            let request_id = format!(
                "osh:{}:{}:{}:{}:{}",
                sandbox_id,
                (seconds * 1000.0) as i64,
                host,
                port.unwrap_or(0),
                decision
            );

            // A name lookup is its own event kind, not a network row carrying a
            // protocol word. The console's DNS facet, "which names did this
            // sandbox ask for", and the live `dns` stream all filter on the kind;
            // a row that says `network` can never answer any of them, and until
            // this branch existed every DNS decision arrived as network/tcp.
            if dns {
                let id = event_id(environment_id, Some(*sandbox_id), Some(&request_id), ts, now);
                let resolved = ts.unwrap_or(now);
                let live = TelemetryEvent::Dns(DnsEvent {
                    id,
                    environment_id,
                    sandbox_id: Some(*sandbox_id),
                    ts: resolved,
                    hostname: host.to_string(),
                    resolver: Some("openshell".to_string()),
                    decision: parse_decision(Some(decision)),
                    binary: event.get("binary").and_then(|v| v.as_str()).map(str::to_string),
                    correlation_id: Some(request_id),
                });
                let mut detail = serde_json::to_value(&live).unwrap_or(serde_json::Value::Null);
                if let serde_json::Value::Object(map) = &mut detail {
                    // The reason is the enforcement's own words and is what an
                    // operator acts on; it has no place in the wire shape, so it
                    // rides in the stored detail only.
                    map.insert("reason".into(), serde_json::json!(reason));
                    // The traffic list reads protocol and channel out of the
                    // detail; a DNS event carries neither on the wire, and a row
                    // without them would be badged as a plain TCP connection.
                    map.insert("protocol".into(), serde_json::json!("dns"));
                    map.insert("channel".into(), serde_json::json!("dns"));
                }
                prepared.push(PreparedEvent {
                    id,
                    ts: resolved,
                    sandbox_id: Some(*sandbox_id),
                    kind: EventKind::Dns,
                    detail,
                    body: None,
                    call: None,
                    live,
                });
                continue;
            }

            let entry = ProxyTrafficEntry {
                dlp: None,
                upstream_ip: None,
                upstream_ip_would: None,
                upstream_host: None,
                router_provider: None,
                router_model: None,
                ts,
                sandbox_id: Some(*sandbox_id),
                host: host.to_string(),
                port,
                protocol: Some("tcp".to_string()),
                method: None,
                path: None,
                status: None,
                duration_ms: None,
                bytes_in: 0,
                bytes_out: 0,
                client: event.get("binary").and_then(|v| v.as_str()).map(str::to_string),
                listener: Some("openshell".to_string()),
                connect: None,
                sni: None,
                app_protocol: None,
                user_agent: None,
                request_id: Some(request_id),
                decision: Some(decision.to_string()),
                would_deny: None,
                reason: if reason.is_empty() { None } else { Some(reason) },
                rule_id: None,
                mcp_session_id: None,
                mcp_method: None,
                mcp_tool: None,
                mcp_server: None,
                request_body: None,
                response_body: None,
                usage: None,
                cache_markers: None,
                llm_auth: None,
            };
            if let Ok(item) = prepare_traffic(&entry, environment_id, now) {
                prepared.push(item);
            }
        }
        if prepared.is_empty() {
            continue;
        }
        let events: Vec<store::BatchEvent<'_>> = prepared
            .iter()
            .map(|item| store::BatchEvent {
                id: item.id,
                environment_id,
                sandbox_id: item.sandbox_id,
                gateway_id: None,
                kind: item.kind,
                source: Some("openshell"),
                ts: item.ts,
                detail: item.detail.clone(),
            })
            .collect();
        let stored = store::insert_batch(&ctx.pool, environment_id, &events)
            .await
            .map_err(ApiError::from)?;
        // Only decisions that were not stored before are announced live.
        if stored > 0 {
            for item in &prepared {
                ctx.bus.publish(LiveEvent::Telemetry(item.live.clone()));
            }
        }
        stored_total += stored as u64;
    }
    Ok(stored_total)
}


/// The model call an access-log line describes, when it carries usage counters.
fn observed_call(
    entry: &ProxyTrafficEntry,
    event_id: Uuid,
    environment_id: Uuid,
) -> Option<crate::services::accounting_service::ObservedCall> {
    use crate::domain::accounting::{AuthKind, ObservedUsage};

    let sandbox_id = entry.sandbox_id?;
    let counters: serde_json::Value = serde_json::from_str(entry.usage.as_deref()?.trim()).ok()?;
    let number = |key: &str| counters.get(key).and_then(serde_json::Value::as_i64).unwrap_or(0);
    let usage = ObservedUsage {
        input: number("i"),
        output: number("o"),
        cache_read: number("cr"),
        cache_write: number("cw"),
        cache_write_1h: number("cw1"),
        reasoning: number("rs"),
    };
    let request = decode_text(entry.request_body.as_deref());
    let response = decode_text(entry.response_body.as_deref());
    let model = request
        .as_deref()
        .and_then(model_of)
        .or_else(|| response.as_deref().and_then(model_of));
    Some(crate::services::accounting_service::ObservedCall {
        id: event_id,
        environment_id,
        sandbox_id,
        host: entry.host.clone(),
        path: entry.path.clone(),
        status: entry.status.unwrap_or(0),
        latency_ms: entry.duration_ms.unwrap_or(0).clamp(0, i64::from(i32::MAX)) as i32,
        model,
        usage,
        cache_markers: entry
            .cache_markers
            .as_deref()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(0),
        auth: AuthKind::parse(entry.llm_auth.as_deref().unwrap_or("")),
        conversation: request.as_deref().and_then(conversation_of),
    })
}

fn decode_text(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = engine
        .decode(raw)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(raw))
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The model a request or an answer names: the first `"model":"…"` in the text.
fn model_of(text: &str) -> Option<String> {
    let at = text.find("\"model\"")?;
    let rest = text[at + 7..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    let name: String = rest.chars().take_while(|c| *c != '"').take(120).collect();
    if name.is_empty() { None } else { Some(name) }
}

/// The conversation a call belongs to: the proxy's own key for a long request, else
/// the `session_id` inside the client's `metadata.user_id` (a JSON string, so its
/// quotes are escaped).
fn conversation_of(text: &str) -> Option<String> {
    for key in ["\"nestbox_conversation\":\"", "session_id\\\":\\\"", "\"session_id\":\""] {
        if let Some(at) = text.find(key) {
            let id: String = text[at + key.len()..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit() || *c == '-')
                .take(64)
                .collect();
            if id.len() >= 8 {
                return Some(id);
            }
        }
    }
    None
}

#[cfg(test)]
mod observed_telemetry_tests {
    use super::*;

    #[test]
    fn the_model_and_the_conversation_are_read_from_a_request() {
        let text = r#"{"model":"claude-opus-5-5","nestbox_conversation":"27a41161-b693-419a-850f-a5f2565bd848"}"#;
        assert_eq!(model_of(text).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(conversation_of(text).as_deref(), Some("27a41161-b693-419a-850f-a5f2565bd848"));
        let escaped = r#"{"metadata":{"user_id":"{\"session_id\":\"aaaaaaaa-bbbb\"}"}}"#;
        assert_eq!(conversation_of(escaped).as_deref(), Some("aaaaaaaa-bbbb"));
        assert_eq!(model_of("{}"), None);
    }
}
