//! Policy, rule, draft, and traffic-window endpoints.

use axum::Json;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::api::principal::Auth;
use crate::domain::egress::{CheckRequest, Decision};
use crate::domain::policy::Policy;
use crate::error::ApiError;
use crate::security::node_token;
use crate::services::policy_service::{self, PolicyDiff, PolicySaveResult};
use crate::state::AppState;

/// List policies.
pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let policies = policy_service::list(&state.services, &auth.principal).await?;
    let global = policy_service::global(&state.services, &auth.principal).await?;
    // The kind is derived from the document, not stored: a policy edited to add
    // a filesystem section becomes an OpenShell policy without a migration.
    let items: Vec<serde_json::Value> = policies
        .iter()
        .map(|policy| {
            let mut value = serde_json::to_value(policy).unwrap_or(serde_json::Value::Null);
            if let serde_json::Value::Object(map) = &mut value {
                map.insert(
                    "kind".into(),
                    serde_json::json!(crate::domain::policy::classify(&policy.spec).as_str()),
                );
                map.insert(
                    "sections".into(),
                    serde_json::json!(crate::domain::policy::document_sections(&policy.spec)),
                );
            }
            value
        })
        .collect();
    Ok(Json(serde_json::json!({
        "items": items,
        "global": global,
    })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePolicyRequest {
    pub name: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    /// The complete four-section document.
    pub spec: serde_json::Value,
    #[serde(default)]
    pub version: Option<i64>,
}

fn default_scope() -> String {
    "sandbox".to_string()
}

/// Create a policy.
pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CreatePolicyRequest>,
) -> Result<(StatusCode, Json<PolicySaveResult>), ApiError> {
    let scope = crate::domain::policy::PolicyScope::parse(&body.scope)
        .ok_or_else(|| ApiError::validation("Scope must be 'sandbox' or 'global'."))?;
    let result = policy_service::create(
        &state.services,
        &auth.principal,
        body.name.trim(),
        scope,
        &body.spec,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// Read one policy.
pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<Policy>, ApiError> {
    Ok(Json(
        policy_service::get(&state.services, &auth.principal, id).await?,
    ))
}

/// The active global override.
pub async fn global(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let policy = policy_service::global(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "policy": policy })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePolicyRequest {
    pub spec: serde_json::Value,
    pub version: i64,
}

/// Replace a policy document.
pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdatePolicyRequest>,
) -> Result<Json<PolicySaveResult>, ApiError> {
    let result = policy_service::update(
        &state.services,
        &auth.principal,
        id,
        body.version,
        &body.spec,
    )
    .await?;
    Ok(Json(result))
}

/// Delete a policy.
pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    policy_service::delete(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct DiffQuery {
    pub against: Option<i64>,
}

/// Diff a policy against an earlier revision.
pub async fn diff(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<PolicyDiff>, ApiError> {
    Ok(Json(
        policy_service::diff(&state.services, &auth.principal, id, query.against).await?,
    ))
}

/// All network rules.
pub async fn list_rules(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rules = policy_service::list_rules(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": rules })))
}

/// Rules attached to one sandbox.
pub async fn sandbox_rules(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rules =
        policy_service::rules_for_sandbox(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "items": rules })))
}

#[derive(Debug, Deserialize)]
pub struct AllowRequest {
    pub host: String,
    pub port: Option<i32>,
    pub protocol: Option<String>,
}

/// Let one destination through for one sandbox, in one step.
pub async fn allow_destination(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<AllowRequest>,
) -> Result<Json<crate::domain::policy::NetworkRule>, ApiError> {
    let rule = policy_service::allow_destination(
        &state.services,
        &auth.principal,
        id,
        &body.host,
        body.port,
        body.protocol.as_deref(),
    )
    .await?;
    Ok(Json(rule))
}

/// Create a network rule.
pub async fn create_rule(
    State(state): State<AppState>,
    auth: Auth,
    Json(mut rule): Json<crate::domain::policy::NetworkRule>,
) -> Result<(StatusCode, Json<crate::domain::policy::NetworkRule>), ApiError> {
    // The id is assigned by the database; a caller-supplied one is ignored so a
    // request cannot overwrite an existing row by guessing its id.
    rule.id = Uuid::nil();
    let created = policy_service::create_rule(&state.services, &auth.principal, rule).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Delete a network rule.
pub async fn delete_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    policy_service::delete_rule(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// All DNS rules.
pub async fn list_dns_rules(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rules = policy_service::list_dns_rules(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": rules })))
}

/// Create a DNS rule.
pub async fn create_dns_rule(
    State(state): State<AppState>,
    auth: Auth,
    Json(mut rule): Json<crate::domain::policy::DnsRule>,
) -> Result<(StatusCode, Json<crate::domain::policy::DnsRule>), ApiError> {
    // Same rule as the network rule: the database assigns the identity.
    rule.id = Uuid::nil();
    let created = policy_service::create_dns_rule(&state.services, &auth.principal, rule).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

/// Delete a DNS rule.
pub async fn delete_dns_rule(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    policy_service::delete_dns_rule(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One sandbox's egress mode.
pub async fn get_egress_mode(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::domain::policy::EgressModeSetting>, ApiError> {
    let setting = policy_service::egress_mode(&state.services, &auth.principal, id).await?;
    Ok(Json(setting))
}

// ---------------------------------------------------------------------------
// Presets, the document schema, and the policy of one sandbox
// ---------------------------------------------------------------------------

/// The preset library.
pub async fn list_templates(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let templates = policy_service::list_templates(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": templates })))
}

/// The body of an operator's own preset.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTemplateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The document the preset produces; validated like every save.
    pub spec: serde_json::Value,
}

/// Save a preset.
pub async fn create_template(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CreateTemplateRequest>,
) -> Result<(StatusCode, Json<crate::domain::policy::PolicyTemplate>), ApiError> {
    let template = policy_service::create_template(
        &state.services,
        &auth.principal,
        &body.name,
        &body.description,
        &body.spec,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(template)))
}

/// The body of an instantiation: which sandbox receives the new policy.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InstantiateRequest {
    /// The sandbox the new policy is assigned to, when it should be applied at
    /// once. Omitted means the policy is created and left unassigned.
    #[serde(default)]
    pub assign_to: Option<Uuid>,
}

/// Turn a preset into a policy.
pub async fn instantiate_template(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<InstantiateRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let result = policy_service::instantiate_template(
        &state.services,
        &auth.principal,
        id,
        body.assign_to,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

/// The document schema the console's editor renders from.
pub async fn schema(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let schema = policy_service::policy_schema(&state.services, &auth.principal)?;
    Ok(Json(schema))
}

/// The body of an assignment.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignPolicyRequest {
    pub policy_id: Uuid,
}

/// Which policy a sandbox carries.
pub async fn get_sandbox_policy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let assignment =
        policy_service::policy_of_sandbox(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "assignment": assignment })))
}

/// Assign a policy to a sandbox, replacing the one it had.
pub async fn put_sandbox_policy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<AssignPolicyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result = policy_service::assign_policy_to_sandbox(
        &state.services,
        &auth.principal,
        id,
        body.policy_id,
    )
    .await?;
    Ok(Json(result))
}

/// Remove a sandbox's policy assignment.
pub async fn delete_sandbox_policy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let result =
        policy_service::unassign_policy_from_sandbox(&state.services, &auth.principal, id)
            .await?;
    Ok(Json(result))
}

/// The body of an egress-mode change.
#[derive(Debug, Deserialize, ToSchema)]
pub struct EgressModeRequest {
    /// `transparent`, `hybrid`, `forward_proxy`, `off_allow_all` or `direct`.
    pub mode: String,
    /// Locator for the sandbox-scoped trust CA, when TLS is to be read.
    #[serde(default)]
    pub trust_ca_ref: Option<String>,
    /// Switch back to the previous mode after this many minutes.
    #[serde(default)]
    pub revert_after_minutes: Option<i32>,
}

/// Set one sandbox's egress mode.
pub async fn put_egress_mode(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<EgressModeRequest>,
) -> Result<Json<crate::domain::policy::EgressModeSetting>, ApiError> {
    let mode = crate::domain::policy::EgressMode::parse(&body.mode).ok_or_else(|| {
        ApiError::validation(
            "An egress mode is transparent, hybrid, forward_proxy, off_allow_all or direct.",
        )
    })?;
    let setting = crate::services::egress_live_service::apply(
        &state.services,
        &auth.principal,
        id,
        mode,
        body.revert_after_minutes,
    )
    .await?;
    Ok(Json(setting))
}

/// Pending rule drafts.
pub async fn list_drafts(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let drafts = policy_service::pending_drafts(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": drafts })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DraftRequest {
    #[serde(default = "default_source")]
    pub source_kind: String,
    pub event: serde_json::Value,
    pub sandbox_id: Option<Uuid>,
}

fn default_source() -> String {
    "network".to_string()
}

/// Draft a rule from an event.
pub async fn draft_from_event(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<DraftRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let draft = policy_service::draft_from_event(
        &state.services,
        &auth.principal,
        &body.source_kind,
        &body.event,
        body.sandbox_id,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(draft)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DecisionRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Approve a draft.
pub async fn approve_draft(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<DecisionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        policy_service::decide_draft(
            &state.services,
            &auth.principal,
            id,
            true,
            body.reason.as_deref(),
        )
        .await?,
    ))
}

/// Reject a draft.
pub async fn reject_draft(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<DecisionRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        policy_service::decide_draft(
            &state.services,
            &auth.principal,
            id,
            false,
            body.reason.as_deref(),
        )
        .await?,
    ))
}

/// The revision history of one policy, newest first.
pub async fn versions(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let rows = policy_service::versions(&state.services, &auth.principal, id).await?;
    Ok(Json(serde_json::json!({ "items": rows })))
}

/// The traffic windows in force, with their mode, channel and expiry.
pub async fn windows(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let windows = policy_service::windows(&state.services, &auth.principal).await?;
    Ok(Json(serde_json::json!({ "items": windows })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct WindowRequest {
    pub sandbox_id: Option<Uuid>,
    #[serde(default = "default_minutes")]
    pub minutes: i32,
    /// Open until somebody closes it; `minutes` is then ignored.
    #[serde(default)]
    pub unlimited: bool,
    /// `bypass` allows and forgets; `audit` allows and records the block.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// `all`, `proxy`, `dns`, `mcp`, `gateway` or `openshell`.
    #[serde(default = "default_channel")]
    pub channel: String,
    pub reason: String,
}

fn default_mode() -> String {
    "bypass".to_string()
}

fn default_channel() -> String {
    "all".to_string()
}

fn default_minutes() -> i32 {
    15
}

/// Open a temporary traffic window.
pub async fn open_window(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<WindowRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let window = policy_service::open_window(
        &state.services,
        &auth.principal,
        body.sandbox_id,
        if body.unlimited { None } else { Some(body.minutes) },
        &body.mode,
        &body.channel,
        body.reason.trim(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(window)))
}

/// Close a window early.
pub async fn close_window(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    policy_service::close_window(&state.services, &auth.principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Decide one egress request on behalf of a node's proxy.
///
/// The caller is a node, not a browser: it presents the node token, so there is
/// no session and no CSRF token. Default deny applies here — a request that
/// cannot be read is refused, never allowed.
///
/// The facts arrive as **headers**, not as a JSON body. Envoy's `ext_authz`
/// HTTP filter forwards a request that keeps the original method and appends the
/// original path to `path_prefix`; it copies only the headers it is configured
/// to copy. So the route accepts any method, and this handler reads
/// `x-nestbox-sandbox`, the authority, and the original path from the route
/// suffix. A decider that required a POST and a body would refuse every real
/// request.
///
/// The status code carries the decision, because that is what the filter reads:
/// `200` for anything that may proceed (`allow` and `audit` differ only in the
/// header, and in whether the proxy logs it as allowed or as an audit hit), and
/// `403` for `deny`. Every answer also repeats the decision in
/// `x-nestbox-decision`, with the reason in `x-nestbox-reason`, so the proxy's
/// access log can record why without parsing the body.
///
/// Known gap, in `api/mod.rs` rather than here: Envoy appends the original path
/// to `/api/v1/internal/policy/check`, so a request for `/` arrives as
/// `/api/v1/internal/policy/check/`, and the registered wildcard route does not
/// match an empty suffix. This handler already treats an empty suffix as `/`;
/// the route needs one more entry (owned by the router, not this file).
pub async fn internal_check(
    State(state): State<AppState>,
    method: Method,
    OriginalUri(original): OriginalUri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // The node identity and its token are checked together: a body or a header
    // that names a node is a claim, and the token is what makes it true.
    let Some(node_id) = header_value(&headers, "x-nestbox-node")
        .and_then(|value| Uuid::parse_str(value.trim()).ok())
    else {
        return Err(ApiError::unauthenticated(
            node_token::NodeTokenError::Missing.message(),
        ));
    };

    let presented = bearer_token(&headers)
        .or_else(|| header_value(&headers, "x-nestbox-node-token"));
    let Some(presented) = presented else {
        return Err(ApiError::unauthenticated(
            node_token::NodeTokenError::Missing.message(),
        ));
    };

    policy_service::authenticate_node(&state.services, node_id, &presented).await?;

    let authority = header_value(&headers, ":authority")
        .or_else(|| header_value(&headers, "host"))
        .unwrap_or_default();
    let (host, authority_port) = split_authority(
        &header_value(&headers, "x-nestbox-host")
            .or_else(|| header_value(&headers, "x-forwarded-host"))
            .unwrap_or(authority),
    );
    let port = header_value(&headers, "x-nestbox-port")
        .and_then(|value| value.trim().parse::<i32>().ok())
        .or(authority_port)
        .unwrap_or(443);
    let protocol = protocol_of(&headers, port);

    if host.trim().is_empty() {
        return Err(ApiError::validation(
            "The check needs the host the sandbox is reaching.",
        ));
    }

    // Who is asking is decided by the connection, not by a header the sandbox
    // could have written: the proxy reports the address the request came from
    // (`x-nestbox-source-ip`, set by the proxy and never by the client), and the
    // runner says which sandbox holds that address. A header naming a sandbox is
    // accepted only when no source address was reported, which is how the tests
    // and a hand-made check speak.
    let sandbox_id = match header_value(&headers, "x-nestbox-source-ip") {
        Some(address) => resolve_source(&state, node_id, &address).await,
        None => header_value(&headers, "x-nestbox-sandbox")
            .or_else(|| header_value(&headers, "x-nestbox-sandbox-id"))
            .and_then(|value| Uuid::parse_str(value.trim()).ok()),
    };

    let destination = host.clone();
    let request = CheckRequest {
        sandbox_id,
        node_id: Some(node_id),
        host,
        port,
        protocol: Some(protocol),
        method: Some(
            header_value(&headers, "x-nestbox-method")
                .unwrap_or_else(|| method.as_str().to_string()),
        ),
        path: Some(original_path(&headers, &original)),
        user_agent: header_value(&headers, "user-agent"),
        mcp_tool: header_value(&headers, "x-nestbox-mcp-tool")
            .or_else(|| header_value(&headers, "mcp-tool")),
        mcp_session_id: header_value(&headers, "mcp-session-id"),
    };

    let outcome = policy_service::check(
        &state.services,
        policy_service::CheckInput {
            node_id,
            request,
            // The proxy only calls us when it has no cached answer, so a call is
            // a cache miss unless the proxy says it is revalidating one.
            cache_hit: header_value(&headers, "x-nestbox-cache-hit")
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("true")),
            reported_hash: header_value(&headers, "x-nestbox-policy-hash")
                .filter(|value| !value.trim().is_empty()),
        },
    )
    .await?;

    let response = &outcome.response;
    let status = if response.decision == Decision::Deny {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::OK
    };

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "x-nestbox-decision",
        HeaderValue::from_static(response.decision.as_str()),
    );
    response_headers.insert(
        "x-nestbox-reason",
        HeaderValue::from_static(response.reason.as_str()),
    );
    if let Some(rule_id) = response.rule_id {
        insert_header(&mut response_headers, "x-nestbox-rule-id", &rule_id.to_string());
    }
    if let Some(window_id) = response.window_id {
        insert_header(
            &mut response_headers,
            "x-nestbox-window-id",
            &window_id.to_string(),
        );
    }
    if let Some(id) = sandbox_id {
        insert_header(&mut response_headers, "x-nestbox-sandbox", &id.to_string());
    }
    insert_header(
        &mut response_headers,
        "x-nestbox-policy-hash",
        &response.policy_hash,
    );
    // The proxy reads this to choose between a blind tunnel and TLS termination.
    // Absent or unknown means tunnel, so a proxy that does not understand the
    // value cannot start inspecting traffic.
    insert_header(
        &mut response_headers,
        "x-nestbox-egress-mode",
        &crate::domain::egress::mode_for_host(&response.egress_mode, &destination),
    );
    insert_header(
        &mut response_headers,
        "x-nestbox-cache-ttl",
        &response.cache_ttl_seconds.to_string(),
    );
    // What the rules would have said while a window or the off mode let this
    // through. The proxy logs it, and a later rule can be drafted from it.
    insert_header(
        &mut response_headers,
        "x-nestbox-would-deny",
        if response.would_deny { "1" } else { "0" },
    );
    if outcome.cache_hit {
        response_headers.insert("x-nestbox-cache-hit", HeaderValue::from_static("true"));
    }

    Ok((status, response_headers, Json(response)).into_response())
}

/// The route prefix the proxy appends the original path to.
const CHECK_PREFIX: &str = "/api/v1/internal/policy/check";

/// One request header as a string, if present and printable.
fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// The token from `Authorization: Bearer …` or the published `Token` form.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = header_value(headers, "authorization")?;
    let (scheme, token) = raw.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") || scheme.eq_ignore_ascii_case("token") {
        Some(token.trim().to_string())
    } else {
        None
    }
}

/// Insert a header, ignoring a value that is not printable ASCII.
///
/// The values here are a UUID, a hex hash, and a number, so this cannot fail in
/// practice; it stays total because a header must never be the reason a decision
/// is dropped.
fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// `host`, and the port if the authority carried one.
///
/// Bare `host:port`, bracketed IPv6 `[::1]:8443`, and a host with no port are
/// all accepted; anything else is treated as a host without a port, which the
/// caller then completes with the protocol default.
fn split_authority(raw: &str) -> (String, Option<i32>) {
    let raw = raw.trim();
    if let Some((inside, after)) = raw
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        let port = after
            .strip_prefix(':')
            .and_then(|value| value.parse::<i32>().ok());
        return (inside.to_string(), port);
    }
    match raw.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !port.is_empty()
                && port.chars().all(|c| c.is_ascii_digit()) =>
        {
            (host.to_string(), port.parse::<i32>().ok())
        }
        _ => (raw.to_string(), None),
    }
}

/// The wire protocol, from the proxy if it says, otherwise from the request.
///
/// This is an HTTP forward proxy: what reaches us is a request, so the fallback
/// is `https` on 443 and `http` on everything else — never `tcp`, which would
/// silently stop a `rest` rule from matching a service on a non-standard port.
/// The proxy says `tcp` when it forwards a `CONNECT` it did not terminate.
fn protocol_of(headers: &HeaderMap, port: i32) -> String {
    if let Some(value) = header_value(headers, "x-nestbox-protocol")
        .or_else(|| header_value(headers, "x-forwarded-proto"))
    {
        let value = value.trim().to_ascii_lowercase();
        if !value.is_empty() {
            return value;
        }
    }
    if port == 443 {
        "https".to_string()
    } else {
        "http".to_string()
    }
}

/// The path the sandbox asked for.
///
/// Prefers an explicit header, then `:path`, and finally the route suffix: if
/// Envoy appended `/v1/chat/completions`, that is the original path, because the
/// prefix is ours.
fn original_path(headers: &HeaderMap, original: &axum::http::Uri) -> String {
    let explicit = header_value(headers, "x-nestbox-path")
        .or_else(|| header_value(headers, "x-envoy-original-path"))
        .or_else(|| header_value(headers, ":path"))
        .filter(|path| !path.trim().is_empty());
    if let Some(path) = explicit {
        return path;
    }

    let suffix = original.path().strip_prefix(CHECK_PREFIX).unwrap_or("");
    let mut path = if suffix.is_empty() {
        "/".to_string()
    } else {
        suffix.to_string()
    };
    if let Some(query) = original.query() {
        path.push('?');
        path.push_str(query);
    }
    path
}

/// The sandbox that holds this address on the sandbox network, if any.
///
/// The runner is asked for every container's address (a cheap read) and the one
/// that matches is looked up by name. An address nobody holds, or a runner that
/// cannot answer, is `None`: the check then refuses with "no sandbox identified",
/// which is the honest fail-closed answer.
async fn resolve_source(state: &AppState, node_id: Uuid, address: &str) -> Option<Uuid> {
    let address = address.trim();
    // Only the node that asked: every node's Docker hands out the same private addresses, so the
    // merged list of all nodes would name the wrong sandbox for an address held on another one.
    let facts = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        state.services.runtime_for(node_id).list(),
    )
    .await
    .ok()?
    .ok()?;
    let name = facts
        .iter()
        .find(|fact| {
            fact.ip == address || fact.network.iter().any(|interface| interface.ip == address)
        })
        .map(|fact| fact.name.clone())?;
    crate::store::postgres::egress::sandbox_id_by_name(&state.services.pool, &name)
        .await
        .ok()
        .flatten()
}

/// Answer a proxy's question about a CONNECT; anything else unmatched is a 404.
///
/// Envoy forwards a CONNECT to its authorization service as a CONNECT whose
/// target is the destination (`api.example.com:443`), which no path route can
/// receive. The decision is the same as for any request: the host and port come
/// from the authority, the sandbox from the source address, and the node must
/// still present its token, so this opens nothing to an unauthenticated caller.
pub async fn connect_or_not_found(
    State(state): State<AppState>,
    method: Method,
    original: OriginalUri,
    headers: HeaderMap,
) -> Response {
    if method != Method::CONNECT {
        return StatusCode::NOT_FOUND.into_response();
    }
    match internal_check(State(state), method, original, headers).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CloneRequest {
    /// The name of the copy.
    pub name: String,
}

/// Copy a policy under a new name (never as a global override).
pub async fn clone_policy(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<CloneRequest>,
) -> Result<(StatusCode, Json<policy_service::PolicySaveResult>), ApiError> {
    let saved = policy_service::clone_policy(&state.services, &auth.principal, id, &body.name).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AssignManyRequest {
    pub sandbox_ids: Vec<Uuid>,
}

/// Assign one policy to several sandboxes.
pub async fn assign_many(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<Uuid>,
    Json(body): Json<AssignManyRequest>,
) -> Result<Json<policy_service::BulkResult>, ApiError> {
    Ok(Json(
        policy_service::assign_policy_to_many(&state.services, &auth.principal, id, &body.sandbox_ids).await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyRulesRequest {
    /// Rules to copy…
    #[serde(default)]
    pub rule_ids: Vec<Uuid>,
    /// …or every rule of this sandbox.
    pub from_sandbox_id: Option<Uuid>,
    /// Sandboxes that receive a copy.
    #[serde(default)]
    pub to_sandbox_ids: Vec<Uuid>,
    /// Instead: one environment-wide rule per source, which applies to every sandbox.
    #[serde(default)]
    pub to_all: bool,
}

/// Copy rules to other sandboxes, or to all of them.
pub async fn copy_rules(
    State(state): State<AppState>,
    auth: Auth,
    Json(body): Json<CopyRulesRequest>,
) -> Result<Json<policy_service::BulkResult>, ApiError> {
    Ok(Json(
        policy_service::copy_rules(
            &state.services,
            &auth.principal,
            &body.rule_ids,
            body.from_sandbox_id,
            &body.to_sandbox_ids,
            body.to_all,
        )
        .await?,
    ))
}
