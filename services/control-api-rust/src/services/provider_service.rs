//! Provider profiles: registration, credential resolution, and a live probe.
//!
//! Why this exists as a service rather than a console-only feature: the sandbox
//! creator needs the same answer to "which endpoint and credential should this
//! workload use". Keeping the rule here means a sandbox created from `curl` and
//! one created from the console get the same environment, which is what makes
//! the two paths trustworthy.

use std::collections::BTreeMap;
use std::time::Duration;

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::model_install::ModelInstall;
use crate::domain::node::Node;
use crate::domain::provider::{ProviderKind, ProviderProfile, ProviderStatus};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::providers as store;

/// What a probe learned about a provider.
///
/// Deliberately does not include the response body: it can contain provider
/// headers and, in a misconfigured endpoint, somebody else's catalogue.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ProbeResult {
    pub reachable: bool,
    pub credential_configured: bool,
    pub status: Option<u16>,
    pub model_count: Option<usize>,
    /// A short, redacted explanation when the probe failed.
    pub detail: String,
}

/// All provider profiles in the environment.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<ProviderProfile>, ApiError> {
    require(principal, Permission::ProviderRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let profiles = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(profiles)
}

/// One provider profile.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<ProviderProfile, ApiError> {
    require(principal, Permission::ProviderRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let profile = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(profile)
}

/// What a create request carries.
pub struct NewProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub endpoint: String,
    pub model: Option<String>,
    /// A locator, never a value.
    pub secret_ref: Option<String>,
    pub labels: serde_json::Value,
}

/// Create the provider profile that makes an installed model selectable.
///
/// Called from the install path, so the profile exists the moment the install
/// starts. Two things follow from that: the endpoint does not answer yet, which
/// is why the status is `pending`, and the profile is not handed to a sandbox
/// until a probe has seen it answer (the inference environment filters on
/// `reachable`).
///
/// Nothing is downloaded and no model server is started in this phase; the
/// install reserves memory and records intent, the server follows in phase 2.
pub async fn create_for_install(
    scope: &mut TenantScope<'_>,
    install: &ModelInstall,
    node: &Node,
) -> Result<ProviderProfile, StoreError> {
    // A container name is a DNS label, so the dots and colons of a model key
    // become hyphens: `qwen3-1.7b` serves at `model-qwen3-1-7b`.
    let host = install.model_key.replace(['.', ':', '/'], "-").to_lowercase();
    let name = format!("local/{}@{}", install.model_key, node.name);
    // 8080 is the port `model_server_images` starts the server on. The profile
    // is created before the container exists, so the number lives in two
    // places; the install probe is what proves they agree.
    //
    // On a joined node the container name means nothing: the control plane is
    // on another machine and reaches the node through the overlay, where the
    // server is published. That port is chosen when the server starts, so the
    // endpoint carries `0` until the install writes the real one
    // (`providers::set_endpoint`).
    let endpoint = if node.is_local {
        format!("http://model-{host}:8080/v1")
    } else {
        let address = overlay_ip(scope, node.id).await?;
        format!("http://{address}:0/v1")
    };
    store::insert_pending_for_install(
        scope,
        &name,
        &endpoint,
        Some(&install.model_key),
        install.node_id,
        install.id,
    )
    .await
}

/// The overlay address of a joined node.
///
/// `domain::node::Node` carries what a node is, not where it sits in the
/// overlay; the address lives in the row the join service wrote and nothing but
/// the WireGuard setup reads it back. Reading it here keeps the node type from
/// growing a field for one caller. A node without an address cannot be reached,
/// so that is an error rather than a placeholder.
async fn overlay_ip(scope: &mut TenantScope<'_>, node_id: Uuid) -> Result<String, StoreError> {
    let address: Option<String> = sqlx::query_scalar(
        "SELECT overlay_ip FROM nodes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(node_id)
    .fetch_optional(scope.connection())
    .await?
    .flatten();
    address.filter(|value| !value.trim().is_empty()).ok_or(StoreError::NotFound)
}

/// Remove the profile an uninstall leaves behind.
///
/// A profile that a sandbox still selects is not deleted silently: the caller
/// checks for use first and refuses with `provider.in_use`. This function is the
/// mechanical removal once that check has passed.
pub async fn delete_for_install(
    scope: &mut TenantScope<'_>,
    profile_id: Uuid,
) -> Result<(), ApiError> {
    ensure_not_in_use(scope, profile_id).await?;
    store::delete(scope, profile_id).await.map_err(ApiError::from)
}

/// Refuse to remove a profile a sandbox still selects.
///
/// A sandbox that points at a deleted profile starts with an environment naming
/// an endpoint nobody maintains, so the refusal names the sandboxes instead of
/// leaving them to fail later. Both delete paths call this: an operator deleting
/// a profile and an uninstall removing what it created face the same rule.
async fn ensure_not_in_use(
    scope: &mut TenantScope<'_>,
    profile_id: Uuid,
) -> Result<(), ApiError> {
    let names = store::sandboxes_using(scope, profile_id)
        .await
        .map_err(ApiError::from)?;
    if names.is_empty() {
        return Ok(());
    }
    Err(ApiError::conflict(
        ErrorCode::PROVIDER_IN_USE,
        format!("The provider profile is used by {}.", names.join(", ")),
    )
    .with_details(serde_json::json!({ "sandboxes": names })))
}

/// Register a provider profile.
///
/// The endpoint scheme is validated here rather than only by the table's check
/// constraint, so the operator gets a message that names the field instead of a
/// database error.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    request: NewProvider,
) -> Result<ProviderProfile, ApiError> {
    require(principal, Permission::ProviderWrite)?;

    let name = request.name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err(ApiError::validation(
            "A provider name must be between 1 and 120 characters.",
        ));
    }
    let endpoint = request.endpoint.trim();
    if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
        return Err(ApiError::validation(
            "A provider endpoint must start with http:// or https://.",
        ));
    }
    if let Some(locator) = request.secret_ref.as_deref() {
        validate_locator(locator)?;
    }
    priority_from_labels(&request.labels)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let profile = store::insert(
        &mut scope,
        store::NewProviderProfile {
            name,
            kind: request.kind,
            endpoint,
            model: request.model.as_deref(),
            secret_ref: request.secret_ref.as_deref(),
            labels: &request.labels,
        },
    )
    .await
    .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "provider.create",
            subject_type: Some("provider"),
            subject_id: Some(profile.id),
            detail: &serde_json::json!({ "name": profile.name, "kind": profile.kind.as_str() }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(profile)
}

/// Change a provider profile.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    expected_version: i64,
    endpoint: String,
    model: Option<String>,
    secret_ref: Option<String>,
    labels: serde_json::Value,
) -> Result<ProviderProfile, ApiError> {
    require(principal, Permission::ProviderWrite)?;

    let endpoint = endpoint.trim().to_string();
    if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
        return Err(ApiError::validation(
            "A provider endpoint must start with http:// or https://.",
        ));
    }
    if let Some(locator) = secret_ref.as_deref() {
        validate_locator(locator)?;
    }
    priority_from_labels(&labels)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let profile = store::update(
        &mut scope,
        id,
        expected_version,
        &endpoint,
        model.as_deref(),
        secret_ref.as_deref(),
        &labels,
    )
    .await
    .map_err(|error| match error {
        crate::error::StoreError::VersionConflict => ApiError::conflict(
            ErrorCode::CONFLICT,
            "This provider was changed by someone else; reload and try again.",
        ),
        crate::error::StoreError::NotFound => ApiError::not_found(
            ErrorCode::PROVIDER_NOT_FOUND,
            "No such provider profile.",
        ),
        other => ApiError::from(other),
    })?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "provider.update",
            subject_type: Some("provider"),
            subject_id: Some(profile.id),
            detail: &serde_json::json!({ "name": profile.name, "version": profile.version }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(profile)
}

/// Remove a provider profile.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let profile = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    ensure_not_in_use(&mut scope, id).await?;
    store::delete(&mut scope, id).await.map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "provider.delete",
            subject_type: Some("provider"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "name": profile.name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Ask the provider whether it answers and whether the credential works.
///
/// The probe is a plain `GET <endpoint>/models`, which every OpenAI-compatible
/// server implements. A failure is reported as a value, not as an error: the
/// operator asked a question and gets an answer.
pub async fn probe(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<ProbeResult, ApiError> {
    require(principal, Permission::ProviderRead)?;
    let profile = get(ctx, principal, id).await?;
    let result = measure(ctx, &profile).await;

    // Persist the answer, because the inference environment filters on it: a
    // status that is only in this response would leave a sandbox pointed at an
    // endpoint the probe just found dead.
    let status = if result.reachable {
        ProviderStatus::Reachable
    } else {
        ProviderStatus::Unreachable
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    store::set_status(&mut scope, id, status)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(result)
}

/// One line of the "test every key" answer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ProbeSummary {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub priority: i64,
    pub reachable: bool,
    pub status: String,
    /// The first thing that went wrong, as the probe saw it.
    pub detail: Option<String>,
}

/// Probe every profile and persist each verdict.
///
/// This is what makes a priority list real: without a pass over all keys, an
/// operator has to open each row to learn which one is dead, and the failover
/// order would be built on stale statuses.
pub async fn probe_all(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<ProbeSummary>, ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let profiles = list(ctx, principal).await?;
    let mut summaries = Vec::with_capacity(profiles.len());
    for profile in profiles {
        let result = measure(ctx, &profile).await;
        let status = if result.reachable {
            ProviderStatus::Reachable
        } else {
            ProviderStatus::Unreachable
        };
        let mut scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        store::set_status(&mut scope, profile.id, status)
            .await
            .map_err(ApiError::from)?;
        scope.commit().await.map_err(ApiError::from)?;
        let priority = priority_of(&profile);
        let kind = profile.kind.as_str().to_string();
        summaries.push(ProbeSummary {
            id: profile.id,
            name: profile.name.clone(),
            kind,
            priority,
            reachable: result.reachable,
            status: status.as_str().to_string(),
            detail: Some(result.detail).filter(|text| !text.trim().is_empty()),
        });
    }
    // Sorted the way they are tried, so the list reads as the failover order.
    summaries.sort_by_key(|entry| (entry.priority, entry.name.clone()));
    Ok(summaries)
}

/// Ask the endpoint whether it answers.
///
/// No database access, so what "reachable" means is decided in one place. An
/// HTTP answer counts as reachable even when it is a 401: the server is up and
/// the credential is the problem, which is a different fix from "nothing is
/// listening".
/// The exit a provider profile is called through, when one is chosen.
///
/// Resolved from the row, not from the caller: a probe and a media render both
/// call a provider, and both must leave from the country the operator chose. A
/// disabled exit is not used — an operator who switched it off expects traffic
/// to stop going through it.
pub async fn exit_url(ctx: &ServiceContext, profile: &ProviderProfile) -> Option<String> {
    let mut scope = crate::store::TenantScope::begin(&ctx.pool, profile.environment_id)
        .await
        .ok()?;
    let proxy = crate::store::postgres::proxies::for_provider(&mut scope, profile.id)
        .await
        .ok()
        .flatten();
    let _ = scope.commit().await;
    proxy.filter(|proxy| proxy.enabled).map(|proxy| proxy.url())
}

/// A client that leaves through the profile's exit, when it has one.
pub async fn client_for(
    ctx: &ServiceContext,
    profile: &ProviderProfile,
    timeout: std::time::Duration,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().timeout(timeout);
    if let Some(url) = exit_url(ctx, profile).await {
        let proxy = reqwest::Proxy::all(&url).map_err(|error| format!("exit proxy: {error}"))?;
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|error| format!("client: {error}"))
}

pub(crate) async fn measure(ctx: &ServiceContext, profile: &ProviderProfile) -> ProbeResult {
    let credential = match profile.secret_ref.as_deref() {
        Some(locator) => match crate::services::secret_service::credential(ctx, profile.environment_id, locator).await {
            Ok(value) => Some(value),
            Err(message) => {
                return ProbeResult {
                    reachable: false,
                    credential_configured: true,
                    status: None,
                    model_count: None,
                    detail: message,
                };
            }
        },
        None => None,
    };

    let url = format!("{}/models", profile.endpoint.trim_end_matches('/'));
    let client = match client_for(ctx, profile, Duration::from_secs(10)).await {
        Ok(client) => client,
        Err(error) => {
            return ProbeResult {
                reachable: false,
                credential_configured: profile.credential_configured,
                status: None,
                model_count: None,
                detail: format!("The probe client could not start: {error}"),
            };
        }
    };

    let mut request = client.get(&url);
    if let Some(value) = credential.as_deref() {
        request = request.bearer_auth(value);
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            if !status.is_success() {
                return ProbeResult {
                    reachable: true,
                    credential_configured: profile.credential_configured,
                    status: Some(status.as_u16()),
                    model_count: None,
                    detail: format!(
                        "The provider answered {status}, so the credential or the endpoint is wrong."
                    ),
                };
            }
            let body: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
            let count = body
                .get("data")
                .and_then(|value| value.as_array())
                .map(|items| items.len());
            ProbeResult {
                reachable: true,
                credential_configured: profile.credential_configured,
                status: Some(status.as_u16()),
                model_count: count,
                detail: "The provider answered and the credential was accepted.".to_string(),
            }
        }
        Err(error) => ProbeResult {
            reachable: false,
            credential_configured: profile.credential_configured,
            status: None,
            model_count: None,
            detail: format!("The provider could not be reached: {error}"),
        },
    }
}

/// The environment variables a sandbox needs to talk to an inference provider.
///
/// Only a pack that needs inference calls this. The rule about *which* profile
/// is used lives here: the environment's default is the first
/// OpenAI-compatible profile by name, so the choice is data and can be changed
/// without a deployment.
///
/// The credential is resolved at this point and never stored. If it cannot be
/// resolved, the sandbox is still created — without inference — because a
/// missing key should not block a workload that does not strictly need one.
pub async fn inference_environment(
    ctx: &ServiceContext,
    environment_id: Uuid,
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let profiles = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let Some(profile) = select_inference_profile(&profiles) else {
        return Ok(BTreeMap::new());
    };

    let mut environment = BTreeMap::new();
    let base_url = profile.endpoint.trim_end_matches('/').to_string();
    environment.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
    environment.insert("OPENAI_API_BASE".to_string(), base_url);
    if let Some(model) = profile.model.clone() {
        environment.insert("HERMES_MODEL".to_string(), model);
    }
    environment.insert("HERMES_PROVIDER".to_string(), "openai".to_string());

    match profile.secret_ref.as_deref() {
        Some(locator) => match crate::services::secret_service::credential(ctx, environment_id, locator).await {
            Ok(value) => {
                environment.insert("OPENAI_API_KEY".to_string(), value);
            }
            Err(reason) => {
                // Logged without the locator: even a path can disclose layout.
                tracing::warn!(
                    provider = %profile.name,
                    reason = %reason,
                    "provider credential could not be resolved; sandbox gets no inference key"
                );
            }
        },
        None => {
            // A model server on this node has no credential, but an OpenAI
            // client refuses to start without a value: Hermes says "not
            // connected to any AI provider" before it sends a request. The
            // placeholder is not a secret and cannot be one; the server it
            // reaches ignores it.
            environment.insert("OPENAI_API_KEY".to_string(), "local".to_string());
        }
    }

    Ok(environment)
}

/// Where agents reach the accounting proxy, when the deployment routes model
/// traffic through it. Unset means agents talk to their gateway directly.
pub fn accounting_proxy_url() -> Option<String> {
    std::env::var("NESTBOX_ACCOUNTING_PROXY_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `environment_for_pack`, plus the routing a named sandbox needs: when the
/// deployment sets `NESTBOX_ACCOUNTING_PROXY_URL`, the agent's model base URLs
/// go through the accounting proxy with the sandbox named in the path.
pub async fn environment_for_sandbox(
    ctx: &ServiceContext,
    environment_id: Uuid,
    pack_key: &str,
    sandbox_name: &str,
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut environment = environment_for_pack(ctx, environment_id, pack_key).await?;
    apply_model_grants(ctx, environment_id, sandbox_name, &mut environment).await;
    if let Some(proxy) = accounting_proxy_url() {
        // The granted models decide which dialect an agent is offered before the
        // base URLs are rewritten: `NESTBOX_MODEL_ROUTER=1` makes the accounting
        // proxy choose the upstream from the request's model (ADR 0057).
        let models = crate::services::model_grant_service::effective_by_name(ctx, environment_id, sandbox_name)
            .await
            .unwrap_or_default();
        if !models.is_empty() {
            let names: Vec<&str> = models.iter().map(|model| model.model_name.as_str()).collect();
            let default = models
                .iter()
                .find(|model| model.is_default)
                .map(|model| model.model_name.clone())
                .unwrap_or_else(|| models[0].model_name.clone());
            environment.insert("NESTBOX_MODEL_ROUTER".into(), "1".into());
            environment.insert("NESTBOX_MODEL_LIST".into(), names.join(","));
            environment.insert("NESTBOX_MODEL_DEFAULT".into(), default);
            // A placeholder, because an OpenAI client refuses to start without one.
            // The router replaces the header with the granted model's credential.
            environment.insert("OPENAI_API_KEY".into(), "nestbox".into());
            environment.insert("ANTHROPIC_AUTH_TOKEN".into(), "nestbox".into());
            if !environment.contains_key("OPENAI_BASE_URL") {
                environment.insert("OPENAI_BASE_URL".into(), proxy.clone());
            }
            if !environment.contains_key("ANTHROPIC_BASE_URL") {
                environment.insert("ANTHROPIC_BASE_URL".into(), proxy.clone());
            }
        }
        crate::domain::agent_config::route_through_accounting(&mut environment, &proxy, sandbox_name);
    }
    Ok(environment)
}

/// Point an agent at the model router when the sandbox has been granted models.
///
/// The grants decide *which* models exist and which one is the default; the
/// proxy decides where each of them is served. So the agent's base URLs become
/// the router's path (`…/s/<sandbox>/r`, with `/v1` for the OpenAI family) and
/// the old gateway endpoint is replaced rather than kept: two base URLs would
/// mean some traffic bypasses the router and the grant list would be a
/// suggestion. Without a proxy URL there is nothing to route through, so only
/// the three `NESTBOX_MODEL_*` keys are set for an agent that wants the list.
///
/// A grant read that fails leaves the sandbox with its mapped model
/// environment rather than without one: a sandbox that cannot start is worse
/// than one running on the gateway's default.
async fn apply_model_grants(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_name: &str,
    environment: &mut BTreeMap<String, String>,
) {
    let models = match super::model_grant_service::effective_by_name(ctx, environment_id, sandbox_name)
        .await
    {
        Ok(models) => models,
        Err(error) => {
            tracing::warn!(code = error.code.0, "model grants could not be read for a sandbox");
            return;
        }
    };
    let Some(first) = models.first() else {
        return;
    };

    let names = models
        .iter()
        .map(|model| model.model_name.clone())
        .collect::<Vec<_>>()
        .join(",");
    let default = models
        .iter()
        .find(|model| model.is_default)
        .unwrap_or(first)
        .model_name
        .clone();

    environment.insert("NESTBOX_MODEL_ROUTER".into(), "1".into());
    environment.insert("NESTBOX_MODEL_LIST".into(), names);
    environment.insert("NESTBOX_MODEL_DEFAULT".into(), default);
    let Some(proxy) = accounting_proxy_url() else {
        return;
    };
    let router = format!("{}/s/{}/r", proxy.trim_end_matches('/'), sandbox_name);
    environment.insert("ANTHROPIC_BASE_URL".into(), router.clone());
    environment.insert("OPENAI_BASE_URL".into(), format!("{router}/v1"));
    // Kept equal to the OpenAI base, as the mapped environment does: some agents
    // read one name and some the other.
    environment.insert("OPENAI_API_BASE".into(), format!("{router}/v1"));
    // Placeholders, not credentials: the router sets the real authorization from
    // the profile the chosen model points at, and an agent refuses to start
    // without a value.
    environment
        .entry("OPENAI_API_KEY".into())
        .or_insert_with(|| "nestbox".into());
    environment
        .entry("ANTHROPIC_AUTH_TOKEN".into())
        .or_insert_with(|| "nestbox".into());
}

/// The environment for a pack, by what it speaks.
///
/// The agents (Hermes, Claude Code, OpenCode, OpenChamber) use the OmniRoute profile
/// when there is one that has answered and has a credential: it answers both the
/// Anthropic and the OpenAI API with a capable model, which a small local model does
/// not. Without one, Hermes falls back to the closest OpenAI-shaped endpoint (a model
/// on this node first, see `select_inference_profile`) and the coding agents get
/// nothing. A pack that needs none gets nothing.
pub async fn environment_for_pack(
    ctx: &ServiceContext,
    environment_id: Uuid,
    pack_key: &str,
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut environment = model_environment_for_pack(ctx, environment_id, pack_key).await?;
    if pack_key == "claude-code" {
        // Claude Code keeps its settings, state and login here, on the persistent home
        // the sandbox is created with.
        environment.insert("CLAUDE_CONFIG_DIR".into(), "/home/nestbox/.agent-state/claude".into());
    }
    if matches!(pack_key, "hermes" | "claude-code" | "opencode" | "openchamber") {
        // The Nestbox MCP hub: its address and the shared token, when the hub is
        // deployed (the token file exists). The token is the same for every agent;
        // it authenticates "an agent of this appliance", and each call is logged
        // by the proxy with the sandbox that made it.
        if let Ok(token) = std::fs::read_to_string(ctx.secret_dir.join("mcp_hub_token")) {
            let token = token.trim();
            if !token.is_empty() {
                environment.insert("NESTBOX_MCP_URL".into(), mcp_hub_url());
                environment.insert("NESTBOX_MCP_TOKEN".into(), token.to_string());
            }
        }
        // The media MCP server authenticates with its own token, so a sandbox
        // that is allowed to render does not also receive the filesystem hub's
        // credential. Both are appliance-scoped, neither is per-sandbox.
        if let Ok(token) = std::fs::read_to_string(ctx.secret_dir.join("media_mcp_token")) {
            let token = token.trim();
            if !token.is_empty() {
                environment.insert("NESTBOX_MEDIA_TOKEN".into(), token.to_string());
                environment.insert("NESTBOX_MEDIA_MCP_URL".into(), media_mcp_url());
            }
        }
    }
    Ok(environment)
}

async fn model_environment_for_pack(
    ctx: &ServiceContext,
    environment_id: Uuid,
    pack_key: &str,
) -> Result<BTreeMap<String, String>, ApiError> {
    match pack_key {
        "hermes" | "claude-code" | "opencode" | "openchamber" => {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id)
                .await
                .map_err(ApiError::from)?;
            let profiles = store::list(&mut scope).await.map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            let Some(profile) = profiles
                .iter()
                .find(|p| p.kind == ProviderKind::Omniroute && p.status == ProviderStatus::Reachable && p.secret_ref.is_some())
            else {
                // No gateway: Hermes falls back to the closest endpoint (a model on
                // this node); the coding agents have nothing to talk to.
                return if pack_key == "hermes" {
                    inference_environment(ctx, environment_id).await
                } else {
                    Ok(BTreeMap::new())
                };
            };
            let key = match &profile.secret_ref {
                Some(locator) => crate::services::secret_service::credential(ctx, environment_id, locator).await.ok(),
                None => None,
            };
            let key = match key {
                Some(value) => value,
                None => {
                    tracing::warn!(provider = %profile.name, "gateway credential could not be resolved; sandbox gets no endpoint");
                    return Ok(BTreeMap::new());
                }
            };
            Ok(crate::domain::agent_config::gateway_environment(&profile.endpoint, profile.model.as_deref(), &key))
        }
        _ => Ok(BTreeMap::new()),
    }
}

/// Reject a locator this phase cannot resolve, at write time.
fn validate_locator(locator: &str) -> Result<(), ApiError> {
    if !(locator.starts_with("file://") || locator.starts_with("db://")) {
        return Err(ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::SECRET_UNRESOLVABLE,
            "A secret locator is file://<path> or db://<name> of a stored key.",
        ));
    }
    if locator.len() <= "file://".len() {
        return Err(ApiError::validation(
            "A file:// locator needs an absolute path.",
        ));
    }
    Ok(())
}


/// Which profile a sandbox should be given, from those the environment has.
///
/// The rule, in one place: an OpenAI-shaped endpoint that has answered, never a
/// pending one. A pending profile is an install whose server does not exist
/// yet, and handing it to an agent would send requests to a hostname that does
/// not resolve.
///
/// Among the ones that qualify, the closest endpoint wins: a model installed on
/// this node costs no egress and needs no credential, a gateway is the next
/// best, and a cloud API is last. Without that order the alphabet decided which
/// model an agent used, which is how a sandbox ended up calling the gateway
/// with a key that was not meant for it.
/// Where a provider sits when nobody said otherwise.
pub const DEFAULT_PRIORITY: i64 = 100;

/// Where one profile sits in the failover order.
///
/// Read from `labels.priority`: lower is tried first, absent means 100. A label
/// rather than a column on purpose — the order is a routing preference, not a
/// fact about the endpoint, and a label needs no migration to change.
pub fn priority_of(profile: &ProviderProfile) -> i64 {
    profile
        .labels
        .get("priority")
        .and_then(|value| value.as_i64())
        .unwrap_or(DEFAULT_PRIORITY)
        .clamp(1, 1000)
}

/// The priority a request asks for, checked.
///
/// A wrong type is a 422 naming the field, not a provider quietly created at the
/// default: "I set 3" and "it is 100" must never be the same answer.
pub fn priority_from_labels(labels: &serde_json::Value) -> Result<Option<i64>, ApiError> {
    match labels.get("priority") {
        None => Ok(None),
        Some(value) => match value.as_i64() {
            Some(number) if (1..=1000).contains(&number) => Ok(Some(number)),
            _ => Err(ApiError::validation(
                "A provider priority is a whole number from 1 (tried first) to 1000 (tried last).",
            )),
        },
    }
}

pub fn select_inference_profile(profiles: &[ProviderProfile]) -> Option<&ProviderProfile> {
    let speaks_openai = |profile: &ProviderProfile| {
        matches!(
            profile.kind,
            ProviderKind::LocalOpenaiCompatible
                | ProviderKind::LocalVllm
                | ProviderKind::OpenaiCompatible
                | ProviderKind::Openai
                // NVIDIA NIM exposes the OpenAI-compatible `/v1` inference
                // surface. It is therefore a valid source for the generic
                // inference clients; the secret still resolves only in this
                // service, never in a catalog response.
                | ProviderKind::Nvidia
                | ProviderKind::Omniroute
        )
    };
    let kind_rank = |profile: &ProviderProfile| match profile.kind {
            ProviderKind::LocalOpenaiCompatible | ProviderKind::LocalVllm => 0,
            ProviderKind::OpenaiCompatible | ProviderKind::Openai => 1,
            ProviderKind::Nvidia => 2,
        ProviderKind::Omniroute => 3,
        _ => 3,
    };
    // Two rules, in this order: only a profile whose last probe answered is
    // eligible (a dead key is no fallback), and the lowest priority among them
    // wins. The kind breaks ties, so a local model still beats a cloud gateway
    // at equal priority.
    let reachable = || {
        profiles
            .iter()
            .filter(|profile| profile.status == ProviderStatus::Reachable)
    };
    reachable()
        .filter(|profile| speaks_openai(profile))
        .min_by_key(|profile| (priority_of(profile), kind_rank(profile)))
        .or_else(|| {
            reachable()
                .min_by_key(|profile| (priority_of(profile), kind_rank(profile)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn profile(name: &str, kind: ProviderKind, status: ProviderStatus) -> ProviderProfile {
        ProviderProfile {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            name: name.to_string(),
            kind,
            endpoint: "http://example.invalid/v1".to_string(),
            model: None,
            secret_ref: None,
            credential_configured: false,
            credential_expiry: None,
            labels: serde_json::json!({}),
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            status,
            node_id: None,
            model_install_id: None,
            last_probed_at: None,
        proxy_id: None,
        }
    }

    #[test]
    fn a_pending_profile_never_reaches_a_sandbox() {
        let profiles = vec![profile(
            "local/qwen@local",
            ProviderKind::LocalOpenaiCompatible,
            ProviderStatus::Pending,
        )];
        assert!(select_inference_profile(&profiles).is_none());
    }

    #[test]
    fn a_model_on_this_node_wins_over_the_gateway() {
        let profiles = vec![
            profile("omniroute", ProviderKind::Omniroute, ProviderStatus::Reachable),
            profile(
                "local/qwen3-1.7b@local",
                ProviderKind::LocalOpenaiCompatible,
                ProviderStatus::Reachable,
            ),
        ];
        let chosen = select_inference_profile(&profiles).expect("a profile");
        assert_eq!(chosen.name, "local/qwen3-1.7b@local");
    }

    #[test]
    fn an_openai_compatible_profile_wins_over_a_reachable_other() {
        let profiles = vec![
            profile("nvidia", ProviderKind::Nvidia, ProviderStatus::Reachable),
            profile(
                "omniroute",
                ProviderKind::Omniroute,
                ProviderStatus::Reachable,
            ),
        ];
        let chosen = select_inference_profile(&profiles).expect("a profile");
        assert_eq!(chosen.name, "nvidia");
    }

    #[test]
    fn a_reachable_nim_profile_is_a_valid_openai_inference_source() {
        let profiles = vec![profile("nvidia-nim", ProviderKind::Nvidia, ProviderStatus::Reachable)];
        assert_eq!(select_inference_profile(&profiles).map(|profile| profile.name.as_str()), Some("nvidia-nim"));
    }

    #[test]
    fn a_reachable_profile_is_better_than_nothing_even_when_it_is_not_openai_shaped() {
        let profiles = vec![
            profile("local/x@local", ProviderKind::LocalOpenaiCompatible, ProviderStatus::Pending),
            profile("anthropic", ProviderKind::Anthropic, ProviderStatus::Reachable),
        ];
        let chosen = select_inference_profile(&profiles).expect("a profile");
        assert_eq!(chosen.name, "anthropic");
    }

    #[test]
    fn an_unreachable_profile_is_never_chosen() {
        let profiles = vec![profile(
            "omniroute",
            ProviderKind::Omniroute,
            ProviderStatus::Unreachable,
        )];
        assert!(select_inference_profile(&profiles).is_none());
    }

    fn with_priority(mut profile: ProviderProfile, priority: i64) -> ProviderProfile {
        profile.labels = serde_json::json!({ "priority": priority });
        profile
    }

    #[test]
    fn a_key_with_a_lower_priority_is_tried_first_even_against_a_local_model() {
        let profiles = vec![
            profile("local/qwen@local", ProviderKind::LocalOpenaiCompatible, ProviderStatus::Reachable),
            with_priority(profile("nvidia-b", ProviderKind::Nvidia, ProviderStatus::Reachable), 1),
        ];
        let chosen = select_inference_profile(&profiles).expect("a profile");
        assert_eq!(chosen.name, "nvidia-b", "an explicit priority beats the kind tie-break");
    }

    #[test]
    fn a_dead_key_is_skipped_for_the_next_one_in_the_order() {
        let profiles = vec![
            with_priority(profile("nvidia-first", ProviderKind::Nvidia, ProviderStatus::Unreachable), 1),
            with_priority(profile("nvidia-second", ProviderKind::Nvidia, ProviderStatus::Reachable), 2),
        ];
        let chosen = select_inference_profile(&profiles).expect("a profile");
        assert_eq!(chosen.name, "nvidia-second");
    }

    #[test]
    fn without_a_priority_a_profile_sits_at_the_default() {
        assert_eq!(priority_of(&profile("x", ProviderKind::Nvidia, ProviderStatus::Reachable)), DEFAULT_PRIORITY);
        assert_eq!(priority_of(&with_priority(profile("x", ProviderKind::Nvidia, ProviderStatus::Reachable), 7)), 7);
    }

    #[test]
    fn a_priority_outside_the_range_is_refused_rather_than_clamped_silently() {
        assert!(priority_from_labels(&serde_json::json!({ "priority": 0 })).is_err());
        assert!(priority_from_labels(&serde_json::json!({ "priority": 1001 })).is_err());
        assert!(priority_from_labels(&serde_json::json!({ "priority": "3" })).is_err());
        assert_eq!(priority_from_labels(&serde_json::json!({ "priority": 3 })).unwrap(), Some(3));
        assert_eq!(priority_from_labels(&serde_json::json!({})).unwrap(), None);
    }

    #[test]
    fn the_status_round_trips_through_its_wire_name() {
        for status in [
            ProviderStatus::Pending,
            ProviderStatus::Reachable,
            ProviderStatus::Unreachable,
        ] {
            assert_eq!(ProviderStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ProviderStatus::parse("something"), None);
    }
}

/// Where an agent reaches the Nestbox MCP hub. A deployment sets it when the hub
/// is not reachable as `mcp-hub` (an OpenShell supervisor cannot resolve that name).
/// Where an agent reaches the media MCP server. Overridable for a deployment
/// where the compose name is not resolvable from the sandbox.
pub fn media_mcp_url() -> String {
    match std::env::var("NESTBOX_MEDIA_MCP_URL") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        // A gateway sandbox has no route on the runtime network; its supervisor
        // reaches the host, where the server's port is published (ADR 0035).
        _ if crate::services::sandbox_service::openshell_runtime() => {
            "http://host.openshell.internal:18089/mcp".to_string()
        }
        _ => "http://media-mcp:8080/mcp".to_string(),
    }
}

pub fn mcp_hub_url() -> String {
    match std::env::var("NESTBOX_MCP_HUB_URL") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ if crate::services::sandbox_service::openshell_runtime() => {
            "http://host.openshell.internal:18088/mcp".to_string()
        }
        _ => "http://mcp-hub:8080/mcp".to_string(),
    }
}
