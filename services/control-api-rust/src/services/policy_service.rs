//! Policy validation, change semantics, rules, and rule drafts.
//!
//! Also the egress decider (`check`): the one place that answers whether one
//! outbound request from one sandbox may proceed. It is here rather than in the
//! proxy so there is one rule set, not two (ADR 0001, 0013, 0022).

use chrono::Utc;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::egress::{
    decide, CheckRequest, CheckResponse, CompiledPolicy, Decision, DecisionReason, WindowChannel,
    WindowMode,
};
use crate::domain::policy::{
    apply_outcome, rules_from_document, validate_document, ApplyOutcome, DnsRule, EgressMode,
    EgressModeSetting, Matcher, NetworkRule, Policy, PolicyAssignment, PolicyScope, PolicySection,
    PolicyTemplate,
};
use crate::error::{ApiError, ErrorCode};
use crate::security::node_token;
use crate::store::TenantScope;
use crate::store::postgres::egress as egress_store;
use crate::store::postgres::policies as store;
use crate::store::postgres::sandboxes as sandbox_store;

/// What the API reports after a save.
///
/// Three outcomes, because there are three real cases (ADR 0008). Returning a
/// boolean would force the console to guess which one it got.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct PolicySaveResult {
    pub policy: Policy,
    pub outcome: ApplyOutcome,
    /// The sections the document touches, so the console can name them.
    pub sections: Vec<String>,
    /// Whether any target sandbox is running, which is what made the outcome
    /// what it is.
    pub targets_running: bool,
}

/// List policies.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<Policy>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policies = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(policies)
}

/// Read one policy.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Policy, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policy = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(policy)
}

/// The revision history of one policy, newest first.
///
/// The rows are written by `store::insert` and `store::update` inside the same
/// transaction as the change, so a version cannot exist without its policy and a
/// policy change cannot exist without its version. This read is what makes the
/// history visible instead of merely stored.
pub async fn versions(
    ctx: &ServiceContext,
    principal: &Principal,
    policy_id: Uuid,
) -> Result<Vec<store::VersionRow>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // Reading the policy first turns "not found" and "no revisions" into two
    // different answers.
    store::get(&mut scope, policy_id).await.map_err(ApiError::from)?;
    let rows = store::list_versions(&mut scope, policy_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// The active global override, if any.
pub async fn global(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Option<Policy>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policy = store::global(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(policy)
}

/// Create a policy.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
    scope_value: PolicyScope,
    spec: &serde_json::Value,
) -> Result<PolicySaveResult, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    if scope_value == PolicyScope::Global {
        // A global policy replaces every sandbox policy, so it needs the higher
        // permission even when the caller may edit ordinary policies.
        require(principal, Permission::PolicyGlobal)?;
    }

    let sections = validate_document(spec).map_err(policy_error)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policy = store::insert(&mut scope, name, scope_value, spec, Some(principal.user_id))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(PolicySaveResult {
        policy,
        outcome: ApplyOutcome::AppliesNow,
        sections: sections.iter().map(|s| s.as_str().to_string()).collect(),
        targets_running: false,
    })
}

/// Replace a policy document, reporting what the save will actually do.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    expected_version: i64,
    spec: &serde_json::Value,
) -> Result<PolicySaveResult, ApiError> {
    require(principal, Permission::PolicyWrite)?;

    let sections = validate_document(spec).map_err(policy_error)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    let existing = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    if existing.scope == PolicyScope::Global {
        require(principal, Permission::PolicyGlobal)?;
    }

    // Whether the startup-section warning applies depends on whether anything is
    // actually running with this policy. Asking the store is what makes the
    // answer true rather than assumed.
    let targets_running = sandbox_store::count_by_status(&mut scope)
        .await
        .map_err(ApiError::from)?
        .iter()
        .any(|(status, count)| *count > 0 && (status == "running" || status == "starting"));

    let outcome = apply_outcome(&sections, targets_running);

    if outcome == ApplyOutcome::Rejected {
        return Err(ApiError::conflict(
            ErrorCode::POLICY_NEEDS_RECREATE,
            "This change cannot be applied to running sandboxes.",
        ));
    }

    let policy = store::update(&mut scope, id, expected_version, spec, Some(principal.user_id))
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(PolicySaveResult {
        policy,
        outcome,
        sections: sections.iter().map(|s| s.as_str().to_string()).collect(),
        targets_running,
    })
}

/// A line-by-line comparison of two documents.
///
/// Returned as a list of changed paths rather than a rendered patch: the console
/// decides how to present it, and a text patch would bake in one presentation.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PolicyDiff {
    pub policy_id: Uuid,
    pub from_version: i64,
    pub to_version: i64,
    pub changes: Vec<DiffEntry>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DiffEntry {
    pub path: String,
    pub before: serde_json::Value,
    pub after: serde_json::Value,
}

/// Diff a policy's stored revision against another revision or its current one.
pub async fn diff(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    against: Option<i64>,
) -> Result<PolicyDiff, ApiError> {
    require(principal, Permission::PolicyRead)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let current = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    let from_version = against.unwrap_or(current.version.saturating_sub(1).max(1));
    let before = store::version_spec(&mut scope, id, from_version).await;
    scope.commit().await.map_err(ApiError::from)?;

    let before = match before {
        Ok(value) => value,
        Err(_) => serde_json::json!({}),
    };

    Ok(PolicyDiff {
        policy_id: id,
        from_version,
        to_version: current.version,
        changes: diff_json("", &before, &current.spec),
    })
}

/// Compare two JSON values and collect the changed leaf paths.
fn diff_json(path: &str, before: &serde_json::Value, after: &serde_json::Value) -> Vec<DiffEntry> {
    use serde_json::Value;

    if before == after {
        return Vec::new();
    }

    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            let mut out = Vec::new();
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                let left = a.get(key).cloned().unwrap_or(Value::Null);
                let right = b.get(key).cloned().unwrap_or(Value::Null);
                out.extend(diff_json(&child, &left, &right));
            }
            out
        }
        _ => vec![DiffEntry {
            path: path.to_string(),
            before: before.clone(),
            after: after.clone(),
        }],
    }
}

/// Delete a policy.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policy = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    if policy.scope == PolicyScope::Global {
        require(principal, Permission::PolicyGlobal)?;
    }
    // A policy that sandboxes still use is a 409 naming them, not a foreign-key 500.
    let users: Vec<String> = sqlx::query_scalar(
        "SELECT s.name FROM sandbox_policies sp JOIN sandboxes s ON s.id = sp.sandbox_id
          WHERE sp.policy_id = $1 ORDER BY s.name",
    )
    .bind(id)
    .fetch_all(scope.connection())
    .await
    .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
    if !users.is_empty() {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!("Sandboxes still use this policy: {}. Assign them another policy first.", users.join(", ")),
        ));
    }
    store::delete(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// All network rules.
pub async fn list_rules(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<NetworkRule>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rules = store::list_rules(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rules)
}

/// What a caller may change about a sandbox's own rules, windows and egress mode.
///
/// An operator or admin may change anything (`PolicyWrite`). A member holds only
/// `SandboxPolicy`, and only for a sandbox they can see: the lookup runs under row-level
/// security, so a sandbox that is not theirs is a plain not-found, not a refusal that
/// confirms it exists. Anything not tied to one sandbox (a shared policy, an
/// environment-wide window) stays `PolicyWrite`.
pub(crate) async fn require_sandbox_scope(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
) -> Result<(), ApiError> {
    if principal.can(Permission::PolicyWrite) {
        return Ok(());
    }
    match sandbox_id {
        Some(id) if principal.can(Permission::SandboxPolicy) => {
            sandbox_store::get(scope, id).await.map_err(ApiError::from)?;
            Ok(())
        }
        _ => Err(ApiError::forbidden(Permission::PolicyWrite.code())),
    }
}

/// The sandbox a stored row belongs to, or `None` for a shared one. The table name is a
/// constant chosen by the caller, never user input.
async fn sandbox_of_row(
    scope: &mut TenantScope<'_>,
    table: &'static str,
    id: Uuid,
) -> Result<Option<Uuid>, ApiError> {
    // A draft names its target by type and id; every other table has a `sandbox_id`.
    let column = if table == "rule_drafts" {
        "CASE WHEN target_type = 'sandbox' THEN target_id END"
    } else {
        "sandbox_id"
    };
    let sql = format!("SELECT {column} FROM {table} WHERE id = $1");
    let found: Option<Option<Uuid>> = sqlx::query_scalar(&sql)
        .bind(id)
        .fetch_optional(scope.connection())
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    Ok(found.flatten())
}

/// Create a network rule.
pub async fn create_rule(
    ctx: &ServiceContext,
    principal: &Principal,
    rule: NetworkRule,
) -> Result<NetworkRule, ApiError> {
    
    validate_rule(&rule)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, if rule.policy_id.is_some() { None } else { rule.sandbox_id }).await?;
    let created = store::insert_rule(&mut scope, &rule)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(created)
}

/// Delete a network rule.
pub async fn delete_rule(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    { let owner = sandbox_of_row(&mut scope, "network_rules", id).await?; require_sandbox_scope(&mut scope, principal, owner).await?; }
    store::delete_rule(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Draft a rule from a live event, without enforcing it (ADR 0011).
/// The rule an approval should create, derived from the event that produced it.
///
/// A blocked request is the case where the useful answer is an allow: the
/// operator saw a red line and wants that destination through. An allowed
/// request that is being reviewed becomes a deny. Hardcoding either way makes
/// "approve" mean the opposite of what the console shows, which is exactly the
/// complaint that nothing could be released from the traffic view.
fn draft_action_for(event: &serde_json::Value) -> &'static str {
    match event.get("decision").and_then(|value| value.as_str()).unwrap_or("") {
        "deny" | "blocked" | "default_deny" => "allow",
        _ => "deny",
    }
}

/// A rule protocol from its wire name, defaulting to `rest`.
fn protocol_from(value: Option<&str>) -> crate::domain::policy::RuleProtocol {
    use crate::domain::policy::RuleProtocol;
    match value.unwrap_or("rest") {
        "http" => RuleProtocol::Http,
        "websocket" => RuleProtocol::Websocket,
        "graphql" => RuleProtocol::Graphql,
        "mcp" => RuleProtocol::Mcp,
        "jsonrpc" => RuleProtocol::Jsonrpc,
        "tcp" => RuleProtocol::Tcp,
        _ => RuleProtocol::Rest,
    }
}

pub async fn draft_from_event(
    ctx: &ServiceContext,
    principal: &Principal,
    source_kind: &str,
    event: &serde_json::Value,
    sandbox_id: Option<Uuid>,
) -> Result<serde_json::Value, ApiError> {
    

    let draft = serde_json::json!({
        "action": draft_action_for(event),
        // The traffic table names the destination `host`; older callers say
        // `destination`. Reading only the second one produced a draft with no
        // host, which an approval turned into a rule that matched nothing.
        "host": event
            .get("host")
            .or_else(|| event.get("destination"))
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
        "port": event.get("port"),
        "protocol": event.get("protocol").and_then(|v| v.as_str()).unwrap_or("rest"),
        "enforcement": "enforced",
        "sandbox_id": sandbox_id,
        "source_event": event,
    });

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, sandbox_id).await?;

    let id = store::insert_draft(
        &mut scope,
        event.get("id").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()),
        source_kind,
        if sandbox_id.is_some() { "sandbox" } else { "policy" },
        sandbox_id,
        &draft,
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({ "id": id, "draft": draft, "status": "pending" }))
}

/// Let one destination through for one sandbox, right now.
///
/// This is the one-click release: the operator saw a blocked request and wants
/// it through, so the rule is created enabled in a single step instead of
/// going through a draft that someone has to find and approve. It is still a
/// normal stored rule: it shows in the sandbox's rule list, can be deleted, and
/// is read by the decider and the OpenShell sync like any other.
///
/// Idempotent: allowing the same host and port again returns the rule that is
/// already there, so a double click does not stack copies.
pub async fn allow_destination(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    host: &str,
    port: Option<i32>,
    protocol: Option<&str>,
) -> Result<NetworkRule, ApiError> {
    
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains(' ') || host.contains('/') {
        return Err(ApiError::validation("A destination needs a plain host name."));
    }
    if let Some(port) = port {
        if !(1..=65535).contains(&port) {
            return Err(ApiError::validation("A port is between 1 and 65535."));
        }
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, Some(sandbox_id)).await?;
    // The caller must be able to see the sandbox: another user's sandbox is not
    // theirs to open, whatever their policy permission.
    sandbox_store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;

    let existing = store::list_rules(&mut scope).await.map_err(ApiError::from)?;
    if let Some(rule) = existing.into_iter().find(|rule| {
        rule.sandbox_id == Some(sandbox_id)
            && rule.enabled
            && rule.action == crate::domain::policy::RuleAction::Allow
            && rule.host.eq_ignore_ascii_case(&host)
            && rule.port == port
    }) {
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(rule);
    }

    let mut matchers = vec![crate::domain::policy::Matcher {
        kind: crate::domain::policy::MatcherKind::Host,
        value: Some(host.clone()),
    }];
    if let Some(port) = port {
        matchers.push(crate::domain::policy::Matcher {
            kind: crate::domain::policy::MatcherKind::Port,
            value: Some(port.to_string()),
        });
    }
    let rule = NetworkRule {
        id: Uuid::nil(),
        environment_id: principal.environment_id,
        policy_id: None,
        sandbox_id: Some(sandbox_id),
        name: match port {
            Some(port) => format!("allow {host}:{port}"),
            None => format!("allow {host}"),
        },
        action: crate::domain::policy::RuleAction::Allow,
        host,
        port,
        protocol: protocol_from(protocol),
        methods: Vec::new(),
        matchers,
        binaries: Vec::new(),
        enforcement: crate::domain::policy::Enforcement::Enforced,
        priority: 100,
        enabled: true,
        version: 1,
    };
    validate_rule(&rule)?;
    let created = store::insert_rule(&mut scope, &rule).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(created)
}

/// Pending drafts.
pub async fn pending_drafts(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<serde_json::Value>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let drafts = store::list_pending_drafts(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(drafts
        .into_iter()
        .map(|draft| {
            serde_json::json!({
                "id": draft.id,
                "source_kind": draft.source_kind,
                "target_type": draft.target_type,
                "target_id": draft.target_id,
                "draft": draft.draft,
                "diff": draft.diff,
                "status": draft.status,
            })
        })
        .collect())
}

/// Approve or reject a draft.
///
/// Approval is the only path that creates a rule, which is what makes the
/// two-step workflow real rather than cosmetic.
pub async fn decide_draft(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    approve: bool,
    reason: Option<&str>,
) -> Result<serde_json::Value, ApiError> {
    

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    { let owner = sandbox_of_row(&mut scope, "rule_drafts", id).await?; require_sandbox_scope(&mut scope, principal, owner).await?; }

    let decided = store::decide_draft(
        &mut scope,
        id,
        approve,
        reason,
        Some(principal.user_id),
    )
    .await
    .map_err(|error| match error {
        crate::error::StoreError::VersionConflict => ApiError::conflict(
            ErrorCode::DRAFT_NOT_PENDING,
            "This draft has already been decided.",
        ),
        other => ApiError::from(other),
    })?;

    if approve {
        let draft = &decided.draft;
        // The draft carries the decision the operator saw in the console: an
        // approval of a blocked request has to allow it, and the protocol has to
        // survive, or a released CONNECT would be stored as a `rest` rule and
        // never match. The matchers are written in step with the flat fields so
        // the compiled OpenShell policy and the egress decider agree.
        let host = draft
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let port = draft
            .get("port")
            .and_then(|v| v.as_i64())
            .and_then(|p| i32::try_from(p).ok());
        let protocol = protocol_from(draft.get("protocol").and_then(|v| v.as_str()));
        let action = match draft.get("action").and_then(|v| v.as_str()) {
            Some("allow") => crate::domain::policy::RuleAction::Allow,
            _ => crate::domain::policy::RuleAction::Deny,
        };
        let mut matchers = Vec::new();
        if !host.is_empty() {
            matchers.push(crate::domain::policy::Matcher {
                kind: crate::domain::policy::MatcherKind::Host,
                value: Some(host.clone()),
            });
        }
        if let Some(port) = port {
            matchers.push(crate::domain::policy::Matcher {
                kind: crate::domain::policy::MatcherKind::Port,
                value: Some(port.to_string()),
            });
        }
        let rule = NetworkRule {
            id: Uuid::nil(),
            environment_id: principal.environment_id,
            policy_id: None,
            sandbox_id: decided.target_id,
            name: if host.is_empty() { "from-event".to_string() } else { host.clone() },
            action,
            host,
            port,
            protocol,
            methods: Vec::new(),
            matchers,
            binaries: Vec::new(),
            enforcement: crate::domain::policy::Enforcement::Enforced,
            priority: 100,
            enabled: true,
            version: 1,
        };
        store::insert_rule(&mut scope, &rule)
            .await
            .map_err(ApiError::from)?;
    }

    scope.commit().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({ "id": decided.id, "status": decided.status }))
}

/// Open a temporary traffic window: the master switch, and observation mode.
///
/// A window is scoped to one channel so a DNS window does not open the web, and
/// carries a mode so observation (allow and record the block) is a different
/// thing from a bypass (allow quietly). Every channel is switchable: an OpenShell
/// window is compiled into the sandbox's own policy in audit mode (ADR 0053).
/// `minutes` of `None` opens the window until somebody closes it.
pub async fn open_window(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
    minutes: Option<i32>,
    mode: &str,
    channel: &str,
    reason: &str,
) -> Result<serde_json::Value, ApiError> {
    if let Some(minutes) = minutes {
        if !(1..=525_600).contains(&minutes) {
            return Err(ApiError::validation(
                "A window lasts 1 to 525600 minutes, or is unlimited.",
            ));
        }
    }
    // An environment may bound how long its rules can be suspended (ADR 0053);
    // the bound is read in its own short transaction, before anything is written.
    {
        let mut probe = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        let bound: Option<i32> = store::max_window_minutes_for_environment(&mut probe)
            .await
            .map_err(ApiError::from)?;
        probe.commit().await.map_err(ApiError::from)?;
        if let Some(bound) = bound {
            if minutes.map(|minutes| minutes > bound).unwrap_or(true) {
                return Err(ApiError::validation(format!(
                    "This environment allows windows of at most {bound} minutes."
                )));
            }
        }
    }
    let mode = WindowMode::parse(mode).ok_or_else(|| {
        ApiError::validation("A window mode is bypass or audit.")
    })?;
    let channel = WindowChannel::parse(channel).ok_or_else(|| {
        ApiError::validation("A window channel is all, proxy, dns, mcp, gateway or openshell.")
    })?;
    if reason.trim().is_empty() {
        return Err(ApiError::validation(
            "A reason is required for a temporary traffic window.",
        ));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, sandbox_id).await?;
    let (id, expires_at) = store::open_window(
        &mut scope,
        sandbox_id,
        if sandbox_id.is_some() { "sandbox" } else { "environment" },
        reason,
        minutes,
        mode.as_str(),
        channel.as_str(),
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // An `openshell` window is compiled into the sandbox's own OpenShell policy
    // (ADR 0053), so the change only reaches the supervisor once it is pushed.
    // In the background: the window is already open in the store, and a sandbox
    // that is not on a gateway must not delay the answer.
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        if let Err(error) = sync_openshell_policies(&ctx2).await {
            tracing::warn!(
                code = error.code.0,
                "OpenShell policies not pushed after a window was opened"
            );
        }
    });

    // A window that changes what a sandbox may reach is a control change the
    // console and every other viewer hear about at once.
    ctx.bus.publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
        sandbox_id,
        environment_id: principal.environment_id,
        what: "window".to_string(),
        detail: serde_json::json!({
            "id": id,
            "mode": mode.as_str(),
            "channel": channel.as_str(),
            "expires_at": expires_at,
        }),
    });

    Ok(serde_json::json!({
        "id": id,
        "expires_at": expires_at,
        "minutes": minutes,
        "mode": mode.as_str(),
        "channel": channel.as_str(),
        "reason": reason,
    }))
}

/// The traffic windows in force, for the master-switch panel.
///
/// One read, so a switch shows the same state the decider is applying: the two
/// come from the same table and the same window.
pub async fn windows(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<store::WindowRow>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = store::list_windows(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// Close a window early.
pub async fn close_window(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    { let owner = sandbox_of_row(&mut scope, "traffic_windows", id).await?; require_sandbox_scope(&mut scope, principal, owner).await?; }
    store::revoke_window(&mut scope, id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    // Closing a window changes what a sandbox may reach exactly like opening one
    // does, so the OpenShell policy is pushed again and every console hears it.
    let ctx2 = ctx.clone();
    tokio::spawn(async move {
        if let Err(error) = sync_openshell_policies(&ctx2).await {
            tracing::warn!(code = error.code.0, "OpenShell policies not pushed after a window closed");
        }
    });
    ctx.bus.publish(crate::domain::telemetry::LiveEvent::SandboxControlChanged {
        sandbox_id: None,
        environment_id: principal.environment_id,
        what: "window".to_string(),
        detail: serde_json::json!({ "closed": id }),
    });
    Ok(())
}

/// Project a policy document into rules, for the editor's read view.
pub fn project_rules(spec: &serde_json::Value) -> Vec<crate::domain::policy::NetworkRuleSeed> {
    rules_from_document(spec)
}

fn policy_error(error: crate::domain::policy::PolicyError) -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::POLICY_INVALID,
        error.to_string(),
    )
    .with_details(serde_json::json!({ "code": error.code() }))
}

/// Validate a rule and put its matchers into the authoritative form.
///
/// The matchers are what the decider reads, so an unvalidated one would be a
/// rule that silently matches nothing. The flat host check stays: a rule the
/// console shows as "host example.com" must not be creatable without one.
fn validate_rule(rule: &NetworkRule) -> Result<(), ApiError> {
    if rule.host.trim().is_empty() {
        return Err(ApiError::validation("A rule needs a host."));
    }
    if rule.host.contains(' ') {
        return Err(ApiError::validation("A host must not contain spaces."));
    }
    if rule.policy_id.is_none() && rule.sandbox_id.is_none() {
        return Err(ApiError::validation(
            "A rule must target a policy or a sandbox.",
        ));
    }
    for matcher in &rule.matchers {
        Matcher::new(matcher.kind, matcher.value.clone()).map_err(|error| {
            ApiError::validation(format!("{error}")).with_details(serde_json::json!({
                "code": error.code(),
            }))
        })?;
    }
    Ok(())
}

/// Sections a document touches, for the console's warning label.
pub fn sections_of(spec: &serde_json::Value) -> Vec<PolicySection> {
    validate_document(spec).unwrap_or_default()
}

/// All DNS rules in this environment.
///
/// These are the names the proxy may resolve; they exist in the schema since
/// 0003 and got their API with the matcher change, so the console can offer
/// them like every other rule kind.
pub async fn list_dns_rules(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<DnsRule>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rules = store::list_dns_rules(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rules)
}

/// Create a DNS rule.
pub async fn create_dns_rule(
    ctx: &ServiceContext,
    principal: &Principal,
    rule: DnsRule,
) -> Result<DnsRule, ApiError> {
    

    let pattern = rule.pattern.trim().to_string();
    if pattern.is_empty() {
        return Err(ApiError::validation("A DNS rule needs a pattern."));
    }
    if pattern.contains(' ') {
        return Err(ApiError::validation("A pattern is one name or suffix."));
    }
    if rule.policy_id.is_none() && rule.sandbox_id.is_none() {
        return Err(ApiError::validation(
            "A DNS rule must target a policy or a sandbox.",
        ));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, if rule.policy_id.is_some() { None } else { rule.sandbox_id }).await?;
    let created = store::insert_dns_rule(
        &mut scope,
        &DnsRule { pattern, ..rule },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(created)
}

/// Delete one DNS rule.
pub async fn delete_dns_rule(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    { let owner = sandbox_of_row(&mut scope, "dns_rules", id).await?; require_sandbox_scope(&mut scope, principal, owner).await?; }
    store::delete_dns_rule(&mut scope, id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// One sandbox's egress mode. Absent is forward proxy, said as a value.
pub async fn egress_mode(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<EgressModeSetting, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let setting = store::egress_mode_for_sandbox(&mut scope, sandbox.id)
        .await
        .map_err(ApiError::from)?
        .unwrap_or(EgressModeSetting {
            sandbox_id: sandbox.id,
            mode: EgressMode::ForwardProxy,
            trust_ca_ref: None,
            enabled: true,
            version: 0,
            changed_at: None,
            revert_at: None,
            revert_to: None,
            needs_recreate: false,
        });
    scope.commit().await.map_err(ApiError::from)?;
    Ok(setting)
}

/// Set one sandbox's egress mode.
pub(crate) async fn set_egress_mode_rebuilding(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    mode: EgressMode,
    trust_ca_ref: Option<String>,
) -> Result<EgressModeSetting, ApiError> {
    
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_sandbox_scope(&mut scope, principal, Some(sandbox_id)).await?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    // A sandbox built from a recipe cannot be recreated in place (its entrypoint and
    // supervisor identity are generated when it is created), so it is made again
    // from its recipe with the new mode: the old one is removed, a new one takes its
    // name. Persistent folders (login, history) stay. The sandbox gets a new id,
    // which the answer carries, so the console can follow it.
    if sandbox.image.starts_with("nestbox-recipe/") {
        scope.commit().await.map_err(ApiError::from)?;
        let rebuilt = crate::services::builder_service::rebuild_with_mode(
            ctx,
            principal,
            sandbox.id,
            mode.as_str(),
        )
        .await?;
        let mut again = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        let setting = store::egress_mode_for_sandbox(&mut again, rebuilt.id)
            .await
            .map_err(ApiError::from)?
            .ok_or_else(|| ApiError::validation("The rebuilt sandbox has no egress setting."))?;
        again.commit().await.map_err(ApiError::from)?;
        return Ok(setting);
    }
    let setting = store::set_egress_mode(
        &mut scope,
        sandbox.id,
        mode,
        trust_ca_ref.as_deref(),
        true,
        None,
        None,
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    // A mode is not a rule the proxy reads per request: it decides whether the
    // sandbox has a proxy at all (`direct`) and whether it trusts the node CA
    // (`transparent`, `hybrid`). Docker fixes both when the container is
    // created, so a running sandbox is recreated — the same operation a mount
    // change uses, with the same visible consequence (its non-persistent files
    // are gone, its persistent roots survive).
    if sandbox.status == crate::domain::sandbox::SandboxStatus::Running {
        if let Err(error) = crate::services::sandbox_service::recreate(ctx, principal, sandbox.id).await {
            tracing::warn!(
                sandbox = %sandbox.name,
                code = error.code.0,
                "egress mode saved, but the sandbox could not be recreated to apply it"
            );
        }
    }
    Ok(setting)
}

/// The name of the policy a new sandbox gets when nobody names one.
///
/// This is the "safe defaults" promise, stated once: a sandbox starts with a
/// policy that denies what it does not need, and the operator can see it by
/// name and replace it by assigning another policy.
pub const SAFE_DEFAULTS_POLICY_NAME: &str = "Safe defaults (best practice)";

/// Apply a preset: turn its document into a policy, named after the template.
///
/// The document is validated like every save, so a template an operator edited
/// badly is refused with the same sentence any hand-written document gets. An
/// optional sandbox receives the new policy instead of its current one.
///
/// Applying a preset twice is not an error and not a second policy: a policy
/// with that name and that exact document is reused, which is what makes
/// "apply the safe defaults" idempotent. A name that is taken by a *different*
/// document gets a counted suffix, so the operator sees their second copy
/// instead of a 409 they cannot act on.
pub async fn instantiate_template(
    ctx: &ServiceContext,
    principal: &Principal,
    template_id: Uuid,
    assign_to: Option<Uuid>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyWrite)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let template = store::get_template(&mut scope, template_id)
        .await
        .map_err(ApiError::from)?;
    let sections = validate_document(&template.spec).map_err(policy_error)?;

    let (name, existing) = policy_name_for(&mut scope, &template.name, &template.spec)
        .await
        .map_err(ApiError::from)?;
    let reused = existing.is_some();
    let policy = match existing {
        Some(policy) => policy,
        None => store::insert(
            &mut scope,
            &name,
            PolicyScope::Sandbox,
            &template.spec,
            Some(principal.user_id),
        )
        .await
        .map_err(ApiError::from)?,
    };
    let assignment = match assign_to {
        Some(sandbox_id) => {
            let sandbox = sandbox_store::get(&mut scope, sandbox_id)
                .await
                .map_err(ApiError::from)?;
            Some(
                store::assign_policy(&mut scope, sandbox.id, policy.id)
                    .await
                    .map_err(ApiError::from)?,
            )
        }
        None => None,
    };
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "policy": policy,
        "template": template.name,
        "outcome": apply_outcome(&sections, false),
        "sections": sections.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>(),
        "assignment": assignment,
        "reused": reused,
    }))
}

/// The name a policy derived from a preset should carry, and the policy that
/// already holds it when the document is the same.
///
/// A preset is applied by pressing a button, so pressing it twice must not be a
/// conflict. The name is the preset's until something else uses it.
async fn policy_name_for(
    scope: &mut TenantScope<'_>,
    base: &str,
    spec: &serde_json::Value,
) -> Result<(String, Option<Policy>), crate::error::StoreError> {
    for attempt in 1..=50u32 {
        let candidate = if attempt == 1 {
            base.to_string()
        } else {
            format!("{base} ({attempt})")
        };
        match store::get_policy_by_name(scope, &candidate).await? {
            None => return Ok((candidate, None)),
            Some(policy) if policy.spec == *spec => return Ok((candidate, Some(policy))),
            Some(_) => continue,
        }
    }
    Ok((format!("{base} ({})", Uuid::new_v4()), None))
}

/// Assign a policy document to a sandbox.
///
/// This is how a sandbox gets away from the safe defaults: pick any policy in
/// this environment and it applies instead.
pub async fn assign_policy_to_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    policy_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = sandbox_store::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    // A policy that does not exist here is a 404 before the row is written:
    // assigning by guessed id must not attach a neighbour's policy.
    store::get(&mut scope, policy_id).await.map_err(ApiError::from)?;
    let assignment = store::assign_policy(&mut scope, sandbox.id, policy_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({ "assignment": assignment }))
}

/// Remove a sandbox's policy assignment.
pub async fn unassign_policy_from_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // The sandbox must be one this caller may see: someone else's is a not-found.
    sandbox_store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;
    store::unassign_policy(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({ "assigned": false }))
}

/// The policy assigned to one sandbox, or `null` when it has none.
pub async fn policy_of_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<Option<PolicyAssignment>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    sandbox_store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;
    let assignment = store::assignment_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(assignment)
}

/// All templates, the preset library the console offers.
pub async fn list_templates(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<PolicyTemplate>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let templates = store::list_templates(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(templates)
}

/// Save an operator's own preset.
///
/// The document is validated before the row exists: a template that cannot
/// produce a valid policy is not stored, because a preset list that lies is
/// worse than a short one.
pub async fn create_template(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
    description: &str,
    spec: &serde_json::Value,
) -> Result<PolicyTemplate, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    if name.trim().is_empty() {
        return Err(ApiError::validation("A template needs a name."));
    }
    validate_document(spec).map_err(policy_error)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let template = store::insert_template(
        &mut scope,
        name.trim(),
        description,
        spec,
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(template)
}

/// The safe-defaults policy this environment carries, for the sandbox-create
/// path and for the console to show what a new sandbox starts with.
pub async fn safe_defaults(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Option<Policy>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let policy = store::get_policy_by_name(&mut scope, SAFE_DEFAULTS_POLICY_NAME)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(policy)
}

/// Give a sandbox its safe-defaults policy.
///
/// Called from the sandbox-create path. Assigning is best-effort: a sandbox
/// that already carries an assignment is left alone (the caller named one), and
/// a failure to assign must not fail the create, because a sandbox without a
/// policy is the same thing it was before this feature.
pub async fn apply_safe_defaults(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
) {
    let mut scope = match TenantScope::begin(&ctx.pool, environment_id).await {
        Ok(scope) => scope,
        Err(error) => {
            tracing::warn!(
                environment_id = %environment_id,
                error = %error,
                "could not open a scope for the safe-defaults assignment"
            );
            return;
        }
    };
    let Some(policy) = store::get_policy_by_name(&mut scope, SAFE_DEFAULTS_POLICY_NAME)
        .await
        .map_err(ApiError::from)
        .ok()
        .flatten()
    else {
        tracing::warn!(
            environment_id = %environment_id,
            "no safe-defaults policy seeded; the sandbox starts without a policy"
        );
        return;
    };
    if store::assignment_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)
        .ok()
        .flatten()
        .is_some()
    {
        // The caller already chose a policy; their choice wins.
        return;
    }
    if let Err(error) = store::assign_policy(&mut scope, sandbox_id, policy.id).await {
        tracing::warn!(
            sandbox_id = %sandbox_id,
            error = %error,
            "the safe-defaults assignment failed"
        );
        return;
    }
    if let Err(error) = scope.commit().await {
        tracing::warn!(
            sandbox_id = %sandbox_id,
            error = %error,
            "the safe-defaults assignment did not commit"
        );
    }
}

/// The document schema the console's editor renders from.
///
/// A schema is not tenant data, but reading it is still an authenticated act:
/// the editor is an operator tool, so the permission is the same one that lets
/// a caller read a policy.
pub fn policy_schema(
    _ctx: &ServiceContext,
    principal: &Principal,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyRead)?;
    Ok(crate::domain::policy::policy_schema())
}

/// Network rules attached to one sandbox.
pub async fn rules_for_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<Vec<NetworkRule>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // Only for a sandbox this caller may see: another user's rules name their hosts.
    sandbox_store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;
    let rules = effective_rules(&mut scope, sandbox_id, principal.environment_id).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rules)
}

/// The rules the decider reads for one sandbox, in the order it reads them.
///
/// One function for the console's view, the decision, and the OpenShell policy
/// sync, so the three can never disagree about what applies.
pub async fn effective_rules(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    environment_id: Uuid,
) -> Result<Vec<NetworkRule>, ApiError> {
    // The effective view: exactly the rules the decider reads for this sandbox,
    // in the order it reads them (stored rules, then those compiled from the
    // governing policy's document). Showing only the sandbox's own rows hid the
    // policy that does most of the work.
    let governing = match store::global(scope).await.map_err(ApiError::from)? {
        Some(global) => Some(global),
        None => match store::assignment_for_sandbox(scope, sandbox_id)
            .await
            .map_err(ApiError::from)?
        {
            Some(assignment) => Some(
                store::get(scope, assignment.policy_id)
                    .await
                    .map_err(ApiError::from)?,
            ),
            None => None,
        },
    };
    let mut rules = crate::domain::policy::rules_for_sandbox_view(
        store::list_rules(scope).await.map_err(ApiError::from)?,
        sandbox_id,
        governing.as_ref().map(|policy| policy.id),
    );
    if let Some(policy) = &governing {
        rules.extend(crate::domain::policy::rules_from_policy(policy));
    }
    let providers = crate::store::postgres::providers::list(scope)
        .await
        .map_err(ApiError::from)?;
    rules.extend(crate::domain::policy::rules_from_providers(&providers));
    rules.extend(attached_mcp_rules(scope, sandbox_id, environment_id).await?);
    rules.extend(openshell_proxy_rules(sandbox_id, environment_id));
    rules.sort_by_key(|rule| rule.priority);
    Ok(rules)
}

// ---------------------------------------------------------------------------
// The egress decider
// ---------------------------------------------------------------------------

/// What the proxy asked, and what it told us about itself.
///
/// The token is not here: it is verified before a request reaches `check`, so a
/// decision can never be taken for a caller that has not been authenticated.
pub struct CheckInput {
    /// The node that is asking, from its token. A decision is only given about
    /// a sandbox that runs on this node.
    pub node_id: Uuid,
    /// The request as the proxy saw it.
    pub request: CheckRequest,
    /// Whether the proxy says it answered this from its own cache. We only see
    /// the calls it did not cache, so this is `false` unless it says otherwise;
    /// recording it keeps the column honest instead of always writing `false`.
    pub cache_hit: bool,
    /// The hash of the rule set the proxy says it is enforcing. Written to
    /// `nodes.policy_hash_reported`, which is what the console compares against
    /// `policy_compilations.hash` to show "not deployed".
    pub reported_hash: Option<String>,
}

/// One decision, with the rule set it came from.
pub struct CheckOutcome {
    pub response: CheckResponse,
    pub cache_hit: bool,
}

/// Verify the bearer token a node presents.
///
/// A service function rather than a handler step so the check cannot be wired to
/// a route that forgets it, and so the store lookup and the file read happen
/// behind one refusal message. Every failure is a 401: a caller must not learn
/// whether a node exists, has a token, or merely presented the wrong one.
pub async fn authenticate_node(
    ctx: &ServiceContext,
    node_id: Uuid,
    presented: &str,
) -> Result<(), ApiError> {
    let locator = egress_store::node_token_locator(&ctx.pool, node_id)
        .await
        .map_err(ApiError::from)?;

    let Some(locator) = locator else {
        tracing::warn!(%node_id, "policy check from a node without a token locator");
        return Err(node_token::NodeTokenError::NotConfigured.to_api());
    };

    node_token::verify(&locator, &ctx.secret_dir, presented).map_err(|error| {
        tracing::warn!(%node_id, reason = ?error, "node token refused");
        error.to_api()
    })
}

/// Decide one egress request and record the decision.
///
/// The order is the rule (see `domain::egress`): an open window allows, then the
/// first matching rule by ascending priority decides, and nothing matching
/// denies. The read of rules and windows, the compilation record, and the
/// evidence row share one transaction, so the row the operator sees later
/// describes the rule set that was actually applied.
///
/// Two cases never reach the rule set, because there is no honest answer to
/// give: a request without a sandbox (the proxy could not map the connection)
/// and a request about a sandbox on another node. Both are refused with
/// `default_deny` and are not written to `policy_checks`, which has no
/// environment to be scoped to — the warning in the log is the evidence.
pub async fn check(ctx: &ServiceContext, input: CheckInput) -> Result<CheckOutcome, ApiError> {
    let request = input.request;
    let cache_hit = input.cache_hit;

    if request.host.trim().is_empty() {
        return Ok(CheckOutcome {
            response: refusal(
                "The proxy sent no host, so no rule can be matched.".to_string(),
            ),
            cache_hit,
        });
    }

    let Some(sandbox_id) = request.sandbox_id else {
        return Ok(CheckOutcome {
            response: refusal(
                "The proxy did not identify a sandbox, so no window or rule applies.".to_string(),
            ),
            cache_hit,
        });
    };

    let Some(owner) = egress_store::sandbox_scope(&ctx.pool, sandbox_id)
        .await
        .map_err(ApiError::from)?
    else {
        tracing::warn!(%sandbox_id, %input.node_id, "policy check for an unknown sandbox");
        return Ok(CheckOutcome {
            response: refusal("The sandbox is not known to this control plane.".to_string()),
            cache_hit,
        });
    };

    if owner.node_id != input.node_id {
        tracing::warn!(
            %sandbox_id,
            asking_node = %input.node_id,
            owning_node = %owner.node_id,
            "policy check from a node that does not run the sandbox"
        );
        return Ok(CheckOutcome {
            response: refusal("The sandbox does not run on the node that asked.".to_string()),
            cache_hit,
        });
    }

    let mut scope = TenantScope::begin(&ctx.pool, owner.environment_id)
        .await
        .map_err(ApiError::from)?;

    // Which policy governs this sandbox: the global override when there is one,
    // otherwise the one assigned to it. Its document is compiled into rules here
    // (`rules_from_policy`), and stored rules are narrowed to the ones that apply
    // to this sandbox, so what the operator edits is what the proxy enforces.
    let governing = match store::global(&mut scope).await.map_err(ApiError::from)? {
        Some(global) => Some(global),
        None => match store::assignment_for_sandbox(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?
        {
            Some(assignment) => Some(
                store::get(&mut scope, assignment.policy_id)
                    .await
                    .map_err(ApiError::from)?,
            ),
            None => None,
        },
    };
    let mut rules = crate::domain::policy::rules_for_sandbox_view(
        store::list_rules(&mut scope).await.map_err(ApiError::from)?,
        sandbox_id,
        governing.as_ref().map(|policy| policy.id),
    );
    if let Some(policy) = &governing {
        rules.extend(crate::domain::policy::rules_from_policy(policy));
    }
    // The model endpoints the operator configured are allowed by that very fact.
    let providers = crate::store::postgres::providers::list(&mut scope)
        .await
        .map_err(ApiError::from)?;
    rules.extend(crate::domain::policy::rules_from_providers(&providers));
    rules.extend(attached_mcp_rules(&mut scope, sandbox_id, owner.environment_id).await?);
    rules.extend(openshell_proxy_rules(sandbox_id, owner.environment_id));
    let windows = egress_store::open_windows(&mut scope, Some(sandbox_id))
        .await
        .map_err(ApiError::from)?;
    let hash = store::compiled_hash(&rules);

    let changed = egress_store::record_compilation(
        &mut scope,
        &hash,
        rules.len() as i64,
        windows.len() as i64,
    )
    .await
    .map_err(ApiError::from)?;
    if changed {
        tracing::info!(
            environment_id = %owner.environment_id,
            hash = %hash,
            rules = rules.len(),
            "compiled a new egress rule set"
        );
    }

    let now = Utc::now();
    let policy = CompiledPolicy {
        environment_id: owner.environment_id,
        hash: hash.clone(),
        compiled_at: now,
        rules,
        windows,
    };
    let mut response = decide(&request, &policy, now);
    // The proxy decides tunnel versus TLS termination from this answer, so the
    // sandbox's own setting has to travel with it. Read through the same scope as
    // the rules: a mode that belonged to another tenant would be a leak, and an
    // unset sandbox keeps the `forward_proxy` default.
    let sandbox_mode = store::egress_mode_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
        .map(|setting| setting.mode);
    response.egress_mode = sandbox_mode
        .map(|mode| mode.as_str().to_string())
        .unwrap_or_else(crate::domain::egress::default_egress_mode);
    // A sandbox whose proxy is switched off allows every connection and records
    // it: the rules still ran for `would_deny`, but they cannot block (ADR 0053).
    let allows_everything = sandbox_mode.map(|mode| mode.allows_everything()).unwrap_or(false);
    if allows_everything {
        response = crate::domain::egress::allow_everything(response);
    }
    // A rule compiled from a document has no row; a decision must not cite an id
    // that points at nothing.
    if response.rule_id == Some(Uuid::nil()) {
        response.rule_id = None;
    }
    // A DNS block is stronger than an allow: a name the operator has said a sandbox
    // must not resolve is not reachable by any network rule or window. (An allow
    // DNS rule adds nothing; the network rules still decide.)
    if response.decision != Decision::Deny && !allows_everything {
        let dns_rules = store::list_dns_rules(&mut scope).await.map_err(ApiError::from)?;
        if let Some(blocked) = crate::domain::policy::dns_rule_for(
            &dns_rules,
            sandbox_id,
            governing.as_ref().map(|policy| policy.id),
            &request.host,
        )
        .filter(|rule| rule.action == crate::domain::policy::RuleAction::Deny)
        {
            response.decision = Decision::Deny;
            response.reason = DecisionReason::Rule;
            response.rule_id = None;
            response.window_id = None;
            response.explanation = format!(
                "The name {} is blocked by the DNS rule {}.",
                request.host, blocked.pattern
            );
        }
    }

    egress_store::record_decision(
        &mut scope,
        egress_store::NewDecision {
            sandbox_id: Some(sandbox_id),
            node_id: Some(input.node_id),
            ts: now,
            host: &request.host,
            port: request.port,
            protocol: request.protocol.as_deref().unwrap_or("tcp"),
            method: request.method.as_deref(),
            path: request.path.as_deref(),
            decision: response.decision.as_str(),
            reason: response.reason.as_str(),
            rule_id: response.rule_id,
            window_id: response.window_id,
            cache_hit,
        },
    )
    .await
    .map_err(ApiError::from)?;

    // The node reports what it enforces. Writing it on every call is what makes
    // a node that returns to the current rule set green again; the update only
    // touches a row whose value actually differs, so `reported_at` still means
    // "when this node changed what it enforces".
    if let Some(reported) = input.reported_hash.as_deref() {
        egress_store::record_reported_hash(&mut scope, input.node_id, reported)
            .await
            .map_err(ApiError::from)?;
    }

    scope.commit().await.map_err(ApiError::from)?;

    Ok(CheckOutcome {
        response,
        cache_hit,
    })
}

/// A refusal that came from the input rather than from a rule set.
///
/// `policy_hash` is empty because no rule set was consulted, and the cache TTL
/// is zero: caching "I could not tell" would pin a mistake for as long as the
/// proxy's TTL, and the sandbox may legitimately appear a second later.
fn refusal(explanation: String) -> CheckResponse {
    CheckResponse {
        decision: Decision::Deny,
        reason: DecisionReason::DefaultDeny,
        rule_id: None,
        window_id: None,
        explanation,
        policy_hash: String::new(),
        cache_ttl_seconds: 0,
        would_deny: false,
        egress_mode: crate::domain::egress::default_egress_mode(),
    }
}

/// Whether each node runs the rule set that is compiled now.
///
/// The console's "deployed" badge is this call, not its own comparison: two
/// hashes read at different times can look equal while a compile happened in
/// between.
pub async fn deployment(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<egress_store::PolicyDeployment, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let deployment = egress_store::deployment(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(deployment)
}



/// The allow rule that lets a gateway sandbox reach the egress proxy.
///
/// Empty on the Docker runtime, where the sandbox reaches `egress-proxy:3128`
/// on its own network and the policy is enforced by the proxy alone.
#[cfg(test)]
mod draft_tests {
    use super::*;

    #[test]
    fn a_blocked_event_proposes_an_allow() {
        let event = serde_json::json!({ "decision": "deny", "destination": "api.example.test" });
        assert_eq!(draft_action_for(&event), "allow");
    }

    #[test]
    fn a_default_deny_proposes_an_allow() {
        let event = serde_json::json!({ "decision": "default_deny" });
        assert_eq!(draft_action_for(&event), "allow");
    }

    #[test]
    fn an_allowed_event_is_tightened_with_a_deny() {
        let event = serde_json::json!({ "decision": "allow" });
        assert_eq!(draft_action_for(&event), "deny");
    }

    #[test]
    fn a_connect_keeps_its_protocol() {
        assert_eq!(protocol_from(Some("tcp")), crate::domain::policy::RuleProtocol::Tcp);
        assert_eq!(protocol_from(None), crate::domain::policy::RuleProtocol::Rest);
    }
}

fn openshell_proxy_rules(sandbox_id: Uuid, environment_id: Uuid) -> Vec<NetworkRule> {
    let mut rules = accounting_proxy_rules(sandbox_id, environment_id);
    rules.extend(gateway_proxy_rules(sandbox_id, environment_id));
    rules
}

/// The allow rule for the accounting proxy, when model traffic is routed there.
fn accounting_proxy_rules(sandbox_id: Uuid, environment_id: Uuid) -> Vec<NetworkRule> {
    match crate::services::provider_service::accounting_proxy_url() {
        Some(url) => crate::domain::policy::rules_for_accounting_proxy(&url, sandbox_id, environment_id),
        None => Vec::new(),
    }
}

fn gateway_proxy_rules(sandbox_id: Uuid, environment_id: Uuid) -> Vec<NetworkRule> {
    if !crate::services::sandbox_service::openshell_runtime() {
        return Vec::new();
    }
    let (host, port) = crate::services::sandbox_service::openshell_proxy_endpoint();
    crate::domain::policy::rules_for_egress_proxy(&host, port, sandbox_id, environment_id)
}

/// Allow rules for the MCP servers attached (and enabled) on one sandbox.
async fn attached_mcp_rules(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    environment_id: Uuid,
) -> Result<Vec<NetworkRule>, ApiError> {
    use crate::store::postgres::mcp as mcp_store;
    let attached = mcp_store::list_attachments(scope, sandbox_id).await.map_err(ApiError::from)?;
    let registry = mcp_store::list_servers(scope).await.map_err(ApiError::from)?;
    let servers: Vec<_> = registry
        .into_iter()
        .filter(|server| attached.iter().any(|a| a.enabled && a.server_key == server.key))
        .collect();
    Ok(crate::domain::policy::rules_from_mcp(&servers, sandbox_id, environment_id))
}

/// Push every OpenShell sandbox's compiled policy to its gateway when it changed.
///
/// Called from the reconcile loop, so a rule, an attachment or a provider edited in
/// the console reaches the supervisor within one pass, and a restart of this process
/// re-sends everything once. The last pushed fingerprint is kept in memory only: it
/// is an optimisation, and losing it costs one idempotent `policy set`.
pub async fn sync_openshell_policies(ctx: &ServiceContext) -> Result<u64, ApiError> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static PUSHED: OnceLock<Mutex<HashMap<Uuid, String>>> = OnceLock::new();
    let pushed = PUSHED.get_or_init(|| Mutex::new(HashMap::new()));

    let facts = ctx
        .runtime
        .list()
        .await
        .map_err(|error| ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, error.to_string()))?;
    let on_gateway: std::collections::HashSet<String> = facts
        .into_iter()
        .filter(|fact| fact.container_name.starts_with("openshell-default--") && fact.status == "running")
        .map(|fact| fact.name)
        .collect();
    if on_gateway.is_empty() {
        return Ok(0);
    }
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
    let mut changed = 0;
    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, name FROM sandboxes WHERE status = 'running'")
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
        let mut work = Vec::new();
        for (id, name) in rows.into_iter().filter(|(_, name)| on_gateway.contains(name)) {
            let rules = effective_rules(&mut scope, id, environment_id).await?;
            let mut compiled = crate::domain::openshell_policy::compile(&rules);
            // A sandbox whose rules are suspended gets the destinations it has
            // been seen reaching, in audit mode, on top of its own entries. The
            // fingerprint covers that addition, so switching in and out of the
            // state pushes a policy on the next pass by itself.
            let suspended = rules_suspended(&mut scope, id).await?;
            if suspended {
                let hosts = suspended_hosts(&mut scope, id, &rules).await?;
                compiled = crate::domain::openshell_policy::allow_hosts(&compiled, &hosts);
            }
            work.push((id, name, compiled, suspended));
        }
        scope.commit().await.map_err(ApiError::from)?;
        for (id, name, compiled, suspended) in work {
            let fingerprint = compiled.fingerprint();
            if pushed.lock().map(|map| map.get(&id) == Some(&fingerprint)).unwrap_or(false) {
                continue;
            }
            match ctx.runtime.set_openshell_policy(&name, &compiled.document).await {
                Ok(()) => {
                    if let Ok(mut map) = pushed.lock() {
                        map.insert(id, fingerprint);
                    }
                    changed += 1;
                    tracing::info!(
                        sandbox = %name,
                        skipped = compiled.skipped.len(),
                        allow_all = suspended,
                        "pushed the OpenShell policy"
                    );
                }
                Err(error) => tracing::warn!(sandbox = %name, error = %error, "could not push the OpenShell policy"),
            }
        }
    }
    Ok(changed)
}

/// Whether the sandbox's own rules are suspended, so the supervisor must let
/// everything through and only record it.
///
/// Two ways in: the egress proxy is switched off (`off_allow_all`), or an open
/// window with channel `all` or `openshell` covers the sandbox. A window on a
/// narrower channel does not suspend the supervisor: it opens one firewall, not
/// this one (ADR 0053).
async fn rules_suspended(scope: &mut TenantScope<'_>, sandbox_id: Uuid) -> Result<bool, ApiError> {
    let mode = store::egress_mode_for_sandbox(scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    if mode.map(|setting| setting.mode.allows_everything()).unwrap_or(false) {
        return Ok(true);
    }
    // `open_windows` filters by the database clock, so an expired window is gone
    // on the same pass that it expired.
    let windows = egress_store::open_windows(scope, Some(sandbox_id))
        .await
        .map_err(ApiError::from)?;
    Ok(windows
        .iter()
        .any(|window| matches!(window.channel, WindowChannel::All | WindowChannel::Openshell)))
}

/// The destinations a suspended sandbox may reach: every host its own rules
/// name, plus every host its telemetry recorded in the last seven days.
///
/// OpenShell has no host wildcard (see `openshell_policy::allow_hosts`), so a
/// policy that allows "everything" has to say what everything is. The rule hosts
/// are what the operator already allowed; the telemetry hosts are what the
/// sandbox actually used while it was still blocked, which is the same set the
/// rule drafts screen offers to turn into rules.
async fn suspended_hosts(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    rules: &[NetworkRule],
) -> Result<Vec<(String, i32)>, ApiError> {
    use crate::store::postgres::traffic as traffic_store;
    let mut hosts: Vec<(String, i32)> = rules
        .iter()
        .filter(|rule| rule.enabled && !rule.host.is_empty() && !rule.host.contains('*'))
        .map(|rule| (rule.host.clone(), rule.port.unwrap_or(443)))
        .collect();
    let now = Utc::now();
    let edges = traffic_store::map_edges(
        scope,
        sandbox_id,
        now - chrono::Duration::days(7),
        now,
        500,
    )
    .await
    .map_err(ApiError::from)?;
    for edge in edges {
        if !edge.host.is_empty() {
            hosts.push((edge.host, edge.port));
        }
    }
    hosts.sort();
    hosts.dedup();
    Ok(hosts)
}



/// A copy of a policy under a new name, never global: a clone is a starting point an
/// operator edits and assigns, not an override that appears by itself.
pub async fn clone_policy(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    name: &str,
) -> Result<PolicySaveResult, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(ApiError::validation("A policy name is 1 to 120 characters."));
    }
    let source = get(ctx, principal, id).await?;
    create(ctx, principal, name, PolicyScope::Sandbox, &source.spec).await
}

/// What a bulk operation did, per target, so a partial failure is visible.
#[derive(Debug, serde::Serialize)]
pub struct BulkResult {
    pub done: usize,
    pub failed: Vec<(Uuid, String)>,
}

/// Assign one policy to several sandboxes.
pub async fn assign_policy_to_many(
    ctx: &ServiceContext,
    principal: &Principal,
    policy_id: Uuid,
    sandbox_ids: &[Uuid],
) -> Result<BulkResult, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    if sandbox_ids.is_empty() || sandbox_ids.len() > 200 {
        return Err(ApiError::validation("Name between 1 and 200 sandboxes."));
    }
    let mut result = BulkResult { done: 0, failed: Vec::new() };
    for sandbox_id in sandbox_ids {
        match assign_policy_to_sandbox(ctx, principal, *sandbox_id, policy_id).await {
            Ok(_) => result.done += 1,
            Err(error) => result.failed.push((*sandbox_id, error.message)),
        }
    }
    Ok(result)
}

/// Copy network rules to other sandboxes, or to every sandbox that exists.
///
/// The source is either the listed rules or every rule of one sandbox. A copy keeps
/// the destination, ports, protocol, methods, binaries and enforcement and carries
/// the target's id. `to_all` means every sandbox that exists now: a rule always
/// belongs to a sandbox or a policy (the database enforces it), so a sandbox created
/// later does not inherit it; for that, put the rules in a policy and assign it.
pub async fn copy_rules(
    ctx: &ServiceContext,
    principal: &Principal,
    rule_ids: &[Uuid],
    from_sandbox_id: Option<Uuid>,
    to_sandbox_ids: &[Uuid],
    to_all: bool,
) -> Result<BulkResult, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let to_sandbox_ids: Vec<Uuid> = if to_all {
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sandboxes")
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
        scope.commit().await.map_err(ApiError::from)?;
        ids
    } else {
        to_sandbox_ids.to_vec()
    };
    if to_sandbox_ids.is_empty() || to_sandbox_ids.len() > 200 {
        return Err(ApiError::validation("Name between 1 and 200 target sandboxes, or choose all."));
    }
    let all = list_rules(ctx, principal).await?;
    let sources: Vec<NetworkRule> = all
        .into_iter()
        .filter(|rule| rule_ids.contains(&rule.id) || (from_sandbox_id.is_some() && rule.sandbox_id == from_sandbox_id))
        .collect();
    if sources.is_empty() {
        return Err(ApiError::validation("There is no rule to copy: name rules, or a sandbox that has some."));
    }
    let targets: Vec<Option<Uuid>> = to_sandbox_ids.iter().map(|id| Some(*id)).collect();
    let mut result = BulkResult { done: 0, failed: Vec::new() };
    for target in targets {
        for source in &sources {
            if target.is_some() && target == source.sandbox_id {
                continue;
            }
            let mut copy = source.clone();
            copy.id = Uuid::nil();
            copy.policy_id = None;
            copy.sandbox_id = target;
            copy.version = 1;
            match create_rule(ctx, principal, copy).await {
                Ok(_) => result.done += 1,
                Err(error) => result.failed.push((source.id, error.message)),
            }
        }
    }
    Ok(result)
}
