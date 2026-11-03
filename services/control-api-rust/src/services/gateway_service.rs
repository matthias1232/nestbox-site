//! Gateways: registry, settings, drift, and sync.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::gateway::{Gateway, SettingsApplyMode, Supervisor, SyncOutcome, SyncResult};
use crate::domain::job::JobKind;
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::gateways as store;

/// List gateways.
pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<Gateway>, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let gateways = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(gateways)
}

/// Read one gateway.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Gateway, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let gateway = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(gateway)
}

/// Replace a gateway's operator configuration.
///
/// This is not a policy change (ADR 0012): it bumps the desired settings version
/// and deliberately leaves the applied version alone, which is what creates the
/// drift the sync button resolves.
pub async fn update_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    expected_version: i64,
    settings: &serde_json::Value,
) -> Result<Gateway, ApiError> {
    require(principal, Permission::GatewayWrite)?;

    if !settings.is_object() {
        return Err(ApiError::validation("Gateway settings must be an object."));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let gateway = store::update_settings(&mut scope, id, expected_version, settings)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "gateway.settings_update",
            subject_type: Some("gateway"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "desired_version": gateway.settings_version }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus.publish(LiveEvent::GatewayHealthChanged {
        gateway_id: gateway.id,
        environment_id: principal.environment_id,
        status: gateway.status.as_str().to_string(),
        drifted: gateway.is_drifted(),
    });

    Ok(gateway)
}

/// Read the operator settings stored for a sandbox's supervisor.
///
/// The console edits these as a document, so the shape belongs to the supervisor
/// and not to this API. Reading is allowed to anyone who may read the gateway,
/// and a sandbox without a supervisor reads as an empty document rather than an
/// error, because "nothing set yet" is a valid answer to "what is set?".
pub async fn sandbox_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let supervisor = store::supervisor_for_sandbox(&mut scope, sandbox_id).await;
    let settings = match supervisor {
        Ok(supervisor) => store::supervisor_settings(&mut scope, supervisor.id).await,
        Err(StoreError::NotFound) => Ok(serde_json::json!({})),
        Err(err) => Err(err),
    }
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(settings)
}

/// Store new operator settings for a sandbox's supervisor.
///
/// This writes the desired document and records who changed it. Delivery to the
/// sandbox's supervisor is a separate step: the row keeps the intent even when
/// the sandbox is not running, so the console never loses an edit.
pub async fn update_sandbox_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    settings: &serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::GatewayWrite)?;

    if !settings.is_object() {
        return Err(ApiError::validation("Supervisor settings must be an object."));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let supervisor = store::supervisor_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    store::set_supervisor_settings(&mut scope, supervisor.id, settings)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "supervisor.settings_update",
            subject_type: Some("supervisor"),
            subject_id: Some(supervisor.id),
            detail: &serde_json::json!({ "sandbox_id": sandbox_id }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // The row keeps the intent; the process needs the document. A sandbox that
    // is not running keeps the row and gets it on the next delivery, so an edit
    // is never lost to a stopped container.
    let delivered = deliver_settings(ctx, &sandbox, settings).await?;

    Ok(serde_json::json!({
        "settings": settings,
        "delivered": delivered,
    }))
}

/// Write the settings document into the sandbox the supervisor reads it from.
///
/// False when the sandbox is not running: the row is the source of truth and the
/// next delivery (or the next instance) picks it up.
async fn deliver_settings(
    ctx: &ServiceContext,
    sandbox: &crate::domain::sandbox::Sandbox,
    settings: &serde_json::Value,
) -> Result<bool, ApiError> {
    if sandbox.status != crate::domain::sandbox::SandboxStatus::Running {
        return Ok(false);
    }
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(settings.to_string());
    let directory = crate::domain::builder::supervisor_state_directory();
    let script = format!(
        "mkdir -p {directory} && printf %s '{encoded}' | base64 -d > {directory}/settings.json",
    );
    let outcome = ctx
        .runtime
        .exec(&sandbox.name, &["sh".to_string(), "-c".to_string(), script], 15)
        .await
        .map_err(|error| {
            ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                format!("The settings could not be delivered: {error}"),
            )
        })?;
    Ok(outcome.exit_code == Some(0))
}

/// Push a gateway's desired configuration and mark it applied.
///
/// Idempotent: syncing a gateway that is already in sync reports
/// `AlreadyInSync` rather than pretending to do work. That is what makes the
/// scheduled run safe to repeat every 20 minutes.
pub async fn sync(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<SyncOutcome, ApiError> {
    require(principal, Permission::GatewayWrite)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let gateway = store::get(&mut scope, id).await.map_err(ApiError::from)?;

    if !gateway.is_drifted() {
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(SyncOutcome {
            gateway_id: gateway.id,
            job_id: None,
            applied_version: gateway.settings_applied_version,
            outcome: SyncResult::AlreadyInSync,
        });
    }

    // A settings change that needs a restart cannot be pretended into effect.
    // The mapping is explicit so the console can say which one happened.
    let requires_restart = gateway
        .settings
        .get("requires_restart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let job = crate::store::postgres::jobs::create(
        &mut scope,
        JobKind::GatewayConfigSync,
        &[gateway.id],
        &serde_json::json!({ "gateway_id": gateway.id, "mode": if requires_restart { SettingsApplyMode::RestartRequired } else { SettingsApplyMode::Hot } }),
        None,
        Some(principal.user_id),
    )
    .await
    .map_err(ApiError::from)?;

    store::mark_applied(&mut scope, gateway.id, gateway.settings_version)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "gateway.sync",
            subject_type: Some("gateway"),
            subject_id: Some(gateway.id),
            detail: &serde_json::json!({ "applied_version": gateway.settings_version }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus.publish(LiveEvent::GatewayHealthChanged {
        gateway_id: gateway.id,
        environment_id: principal.environment_id,
        status: gateway.status.as_str().to_string(),
        drifted: false,
    });

    Ok(SyncOutcome {
        gateway_id: gateway.id,
        job_id: Some(job.id),
        applied_version: gateway.settings_version,
        outcome: if requires_restart {
            SyncResult::RestartRequired
        } else {
            SyncResult::Queued
        },
    })
}

/// Every gateway that is out of step, for the sync job.
pub async fn drifted(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<Gateway>, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let gateways = store::drifted(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(gateways)
}

/// List supervisors.
pub async fn supervisors(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<Supervisor>, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let supervisors = store::list_supervisors(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(supervisors)
}

/// Register a supervisor for a sandbox.
///
/// A supervisor is how the console answers "who watches this sandbox". The
/// gateway is the one that accepts placement; the sandbox must exist in this
/// environment, and the name is a slug so it can be a label on the node as
/// well.
pub async fn create_supervisor(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
    sandbox_id: Option<Uuid>,
) -> Result<Supervisor, ApiError> {
    require(principal, Permission::GatewayWrite)?;

    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name.as_bytes()[0].is_ascii_lowercase();
    if !valid {
        return Err(ApiError::validation(
            "A supervisor name is a lowercase slug of letters, digits, and hyphens.",
        ));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    let gateway = store::list(&mut scope)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .find(|gateway| gateway.accepts_placement())
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::GATEWAY_NOT_READY,
                "No gateway is healthy and in sync; a supervisor cannot be registered.",
            )
        })?;

    if let Some(id) = sandbox_id {
        crate::store::postgres::sandboxes::get(&mut scope, id)
            .await
            .map_err(|error| match error {
                StoreError::NotFound => ApiError::not_found(
                    ErrorCode::SANDBOX_NOT_FOUND,
                    "No such sandbox in this environment.",
                ),
                other => ApiError::from(other),
            })?;
    }

    let supervisor = store::insert_supervisor(
        &mut scope,
        gateway.id,
        sandbox_id,
        name,
        "running",
    )
    .await
    .map_err(|error| match error {
        StoreError::UniqueViolation(_) => ApiError::conflict(
            ErrorCode::RECIPE_INVALID,
            format!("A supervisor named '{name}' already exists on this gateway."),
        ),
        other => ApiError::from(other),
    })?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "supervisor.create",
            subject_type: Some("supervisor"),
            subject_id: Some(supervisor.id),
            detail: &serde_json::json!({
                "name": supervisor.name,
                "gateway": gateway.name,
                "sandbox_id": sandbox_id,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;
    Ok(supervisor)
}

/// Create the supervisor that belongs to a sandbox.
///
/// Every sandbox gets one, and its token is generated here: the plaintext goes
/// into the sandbox environment, the hash into the row. A reload does not change
/// it, so a supervisor that restarts keeps reporting for the same row.
pub async fn create_for_sandbox(
    scope: &mut TenantScope<'_>,
    gateway_id: Uuid,
    sandbox_id: Uuid,
    name: &str,
) -> Result<(Supervisor, String), StoreError> {
    use sha2::{Digest, Sha256};

    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let supervisor =
        store::insert_supervisor_with_token(scope, gateway_id, sandbox_id, name, &hash).await?;
    // The sandbox points at its supervisor, so a list can show who watches whom.
    sqlx::query("UPDATE sandboxes SET supervisor_id = $1 WHERE id = $2")
        .bind(supervisor.id)
        .bind(sandbox_id)
        .execute(scope.connection())
        .await?;
    Ok((supervisor, token))
}

/// The two warnings a supervisor raises for a situation that is normal here.
///
/// * no model gateway configured: the agent signs in with its own account, or
///   the recipe has no agent, so there is nothing to reach;
/// * no policy hash delivered: a sandbox on the Docker runtime is governed by
///   the egress proxy, which no supervisor is told about.
///
/// Counting them made every such sandbox permanently "degraded", which hides the
/// day something is really wrong. The checks stay in the report and on the page.
fn is_expected_warning(check: &crate::domain::gateway::SupervisorCheck) -> bool {
    (check.key == "model_reachable" && check.detail.starts_with("no OPENAI_BASE_URL"))
        || (check.key == "policy_drift" && check.detail.starts_with("no desired policy hash"))
}

/// The status to store for a report: `degraded` only when a warning is not one of
/// the expected ones.
pub fn effective_status(
    report: &crate::domain::gateway::SupervisorReport,
) -> crate::domain::gateway::SupervisorStatus {
    use crate::domain::gateway::SupervisorStatus;
    if report.status != SupervisorStatus::Degraded {
        return report.status;
    }
    let unexpected = report
        .checks
        .iter()
        .any(|check| check.state != "ok" && !is_expected_warning(check));
    if unexpected { SupervisorStatus::Degraded } else { SupervisorStatus::Running }
}

/// Record one report from a supervisor.
///
/// The caller is the process in the sandbox, not a browser: it presents the
/// token that was generated when its supervisor row was created, and the token
/// decides which row it may report for. A token without a row, or a row other
/// than the one in the path, is refused before anything is written.
pub async fn record_report(
    ctx: &ServiceContext,
    token: &str,
    supervisor_id: Uuid,
    mut report: crate::domain::gateway::SupervisorReport,
) -> Result<Supervisor, ApiError> {
    use sha2::{Digest, Sha256};

    report.status = effective_status(&report);

    if token.trim().is_empty() {
        return Err(ApiError::unauthenticated("The supervisor token was refused."));
    }
    let mut hasher = Sha256::new();
    hasher.update(token.trim().as_bytes());
    let hash: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();

    let found = store::find_by_token_hash(&ctx.pool, &hash)
        .await
        .map_err(ApiError::from)?;
    let Some((id, environment_id, _sandbox)) = found else {
        return Err(ApiError::unauthenticated("The supervisor token was refused."));
    };
    if id != supervisor_id {
        return Err(ApiError::forbidden("supervisor.report"));
    }

    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let supervisor = store::record_report(&mut scope, id, &report)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(supervisor)
}

/// Bind a registered supervisor to a sandbox.
///
/// The sandbox must exist in this environment; the row keeps its name and
/// gateway, so re-binding after a sandbox is recreated does not create a second
/// supervisor nobody meant to make.
pub async fn bind_supervisor(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    sandbox_id: Uuid,
) -> Result<Supervisor, ApiError> {
    require(principal, Permission::GatewayWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(
                ErrorCode::SANDBOX_NOT_FOUND,
                "No such sandbox in this environment.",
            ),
            other => ApiError::from(other),
        })?;

    let supervisor = store::set_supervisor_sandbox(&mut scope, id, sandbox_id)
        .await
        .map_err(ApiError::from)?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "supervisor.bind",
            subject_type: Some("supervisor"),
            subject_id: Some(supervisor.id),
            detail: &serde_json::json!({ "sandbox_id": sandbox_id }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;
    Ok(supervisor)
}

/// The cluster graph.
///
/// Nodes are gateways and sandboxes, and an edge means "this gateway carries
/// this sandbox". A sandbox with no gateway is shown unconnected rather than
/// given a fake edge, because "not placed" is a real state an operator needs to
/// see.
pub async fn topology(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    let gateways = store::list(&mut scope).await.map_err(ApiError::from)?;
    let sandboxes = crate::store::postgres::sandboxes::topology_rows(&mut scope)
        .await
        .map_err(ApiError::from)?;
    let supervisors = store::list_supervisors(&mut scope)
        .await
        .map_err(ApiError::from)?;
    let clustered = crate::store::postgres::environments::list_clusters(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for cluster in &clustered {
        nodes.push(serde_json::json!({
            "id": cluster.id,
            "kind": "supervisor",
            "name": cluster.name,
            "status": "running",
        }));
    }

    // A registered supervisor is a node of its own: it is what the operator
    // created to watch a sandbox, and the graph should show the thing they made
    // rather than only the cluster it belongs to.
    for supervisor in &supervisors {
        nodes.push(serde_json::json!({
            "id": supervisor.id,
            "kind": "supervisor",
            "name": supervisor.name,
            "status": supervisor.status.as_str(),
            "sandbox_id": supervisor.sandbox_id,
        }));
        edges.push(serde_json::json!({
            "from": supervisor.gateway_id,
            "to": supervisor.id,
            "kind": "hosts",
        }));
        if let Some(sandbox_id) = supervisor.sandbox_id {
            edges.push(serde_json::json!({
                "from": supervisor.id,
                "to": sandbox_id,
                "kind": "hosts",
            }));
        }
    }

    for gateway in &gateways {
        nodes.push(serde_json::json!({
            "id": gateway.id,
            "kind": "gateway",
            "name": gateway.name,
            "status": gateway.status.as_str(),
            "drifted": gateway.is_drifted(),
            "capacity": gateway.capacity,
            "used": gateway.used,
        }));
        edges.push(serde_json::json!({
            "from": gateway.cluster_id,
            "to": gateway.id,
            "kind": "hosts",
        }));
    }

    for sandbox in sandboxes {
        nodes.push(serde_json::json!({
            "id": sandbox.id,
            "kind": "sandbox",
            "name": sandbox.name,
            "status": sandbox.status,
            "placed": sandbox.gateway_id.is_some(),
            // The node the sandbox runs on. A sandbox whose node is unknown
            // reports null rather than being left out: "not placed" and "no
            // node" are different facts.
            "node_id": sandbox.node_id,
            "node_name": sandbox.node_name,
        }));
        if let Some(gateway) = sandbox.gateway_id {
            edges.push(serde_json::json!({
                "from": gateway,
                "to": sandbox.id,
                "kind": "hosts",
            }));
        }
    }

    Ok(serde_json::json!({ "nodes": nodes, "edges": edges }))
}

/// Refuse an action when no gateway is healthy, with an explanation.
pub fn require_healthy(gateways: &[Gateway]) -> Result<&Gateway, ApiError> {
    gateways
        .iter()
        .find(|gateway| gateway.accepts_placement())
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::GATEWAY_NOT_READY,
                "No gateway is healthy and in sync; placement is paused.",
            )
        })
}

/// Queue one whitelisted action for a sandbox's supervisor.
///
/// The control plane does not run the action: it writes one JSON command into
/// `~/.nestbox-supervisor/commands/`, and the supervisor answers in `results/`
/// on its next tick. The whitelist stays in the process that executes it, and a
/// sandbox that is not running is refused instead of being written to.
pub async fn queue_supervisor_action(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    if !matches!(kind, "apply_policy" | "restart_agent" | "run_check") {
        return Err(ApiError::validation(
            "A supervisor action is apply_policy, restart_agent, or run_check.",
        ));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let supervisor = store::supervisor_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    if sandbox.status != crate::domain::sandbox::SandboxStatus::Running {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!(
                "A supervisor action needs a running sandbox; this one is '{}'.",
                sandbox.status.as_str()
            ),
        ));
    }

    let command_id = Uuid::new_v4();
    let document = serde_json::json!({
        "id": command_id.to_string(),
        "type": kind,
        "payload": payload,
        "issued_at": chrono::Utc::now(),
    });
    // The document goes in base64: the payload is operator JSON, and a shell
    // that sees quotes in it is a second parser with its own rules.
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(document.to_string());
    let directory = crate::domain::builder::supervisor_state_directory();
    let script = format!(
        "mkdir -p {directory}/commands && printf %s '{encoded}' | base64 -d > {directory}/commands/{command_id}.json",
    );
    let outcome = ctx
        .runtime
        .exec(
            &sandbox.name,
            &["sh".to_string(), "-c".to_string(), script],
            15,
        )
        .await
        .map_err(|error| {
            ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NODE_RUNTIME_UNAVAILABLE,
                format!("The action could not be delivered: {error}"),
            )
        })?;
    if outcome.exit_code != Some(0) {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!(
                "The action could not be written into the sandbox: {}",
                outcome.output.trim()
            ),
        ));
    }

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "supervisor.action",
            subject_type: Some("supervisor"),
            subject_id: Some(supervisor.id),
            detail: &serde_json::json!({
                "sandbox_id": sandbox_id,
                "action": kind,
                "command_id": command_id,
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    Ok(serde_json::json!({
        "command_id": command_id,
        "action": kind,
        "queued": true,
    }))
}

/// Give every sandbox that has none its supervisor, and report the real state of
/// the ones that run under an OpenShell gateway.
///
/// Called from the reconcile loop. Idempotent: a sandbox that already has a
/// supervisor row is not touched, so an existing one keeps its token and settings.
/// Cross-tenant maintenance like the other boot routines: environments are listed
/// unscoped and every write happens inside its tenant scope.
pub async fn ensure_supervisors(ctx: &ServiceContext) -> Result<u64, ApiError> {
    let facts = ctx.runtime.list().await.unwrap_or_default();
    let on_gateway: std::collections::HashMap<String, crate::domain::runtime::RuntimeFacts> = facts
        .into_iter()
        .filter(|fact| fact.container_name.starts_with("openshell-default--"))
        .map(|fact| (fact.name.clone(), fact))
        .collect();
    // With the Docker runtime there is no gateway to ask. A leftover OpenShell
    // container with the same name must not turn a healthy sandbox's supervisor
    // red every thirty seconds.
    let on_gateway = if crate::services::sandbox_service::openshell_runtime() {
        on_gateway
    } else {
        std::collections::HashMap::new()
    };
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
    let mut changed = 0;
    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let missing: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT s.id, s.gateway_id, s.name FROM sandboxes s
              WHERE s.gateway_id IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM supervisors v WHERE v.sandbox_id = s.id)",
        )
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        for (sandbox_id, gateway_id, name) in missing {
            let name = format!("sup-{name}");
            if create_for_sandbox(&mut scope, gateway_id, sandbox_id, &name).await.is_ok() {
                changed += 1;
            }
        }
        sqlx::query(
            "UPDATE sandboxes s SET supervisor_id = v.id FROM supervisors v
              WHERE v.sandbox_id = s.id AND s.supervisor_id IS NULL",
        )
        .execute(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        let rows: Vec<(Uuid, String, bool)> = sqlx::query_as(
            "SELECT v.id, s.name, v.reported IS NULL FROM supervisors v JOIN sandboxes s ON s.id = v.sandbox_id",
        )
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        for (supervisor_id, sandbox_name, never_reported) in rows {
            let Some(fact) = on_gateway.get(&sandbox_name) else {
                // Not on the gateway and never heard from: there is no
                // supervisor to be "failed". Say so ("unknown") instead of
                // leaving the red state a vanished gateway once wrote.
                if never_reported {
                    sqlx::query(
                        "UPDATE supervisors SET status = 'unknown' WHERE id = $1 AND status = 'failed'",
                    )
                    .bind(supervisor_id)
                    .execute(scope.connection())
                    .await
                    .map_err(|error| ApiError::from(StoreError::Database(error)))?;
                }
                continue;
            };
            let status = match fact.supervisor_status.as_str() {
                "running" | "degraded" | "stopped" | "starting" => fact.supervisor_status.as_str(),
                "missing" => "failed",
                _ => continue,
            };
            // The supervisor ships with the gateway, so its version is the gateway's.
            let version = Some(fact.supervisor_image.as_str()).filter(|version| !version.is_empty());
            sqlx::query(
                "UPDATE supervisors SET status = $2, version = COALESCE($3, version), last_seen = now()
                  WHERE id = $1",
            )
            .bind(supervisor_id)
            .bind(status)
            .bind(version)
            .execute(scope.connection())
            .await
            .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        }
        scope.commit().await.map_err(ApiError::from)?;
    }
    Ok(changed)
}

/// What the OpenShell gateway and the supervisor say about one sandbox, next to the
/// supervisor row Nestbox keeps for it.
///
/// A read: the boundary state, the policy being enforced, the recent log. A sandbox
/// that is not on an OpenShell gateway answers 409 with the reason, because "no
/// supervisor to show" is a fact about that sandbox, not an empty page.
pub async fn supervisor_view(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::GatewayRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = crate::store::postgres::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(|error| match error {
            StoreError::NotFound => ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "No such sandbox in this environment."),
            other => ApiError::from(other),
        })?;
    let supervisor = store::supervisor_for_sandbox(&mut scope, sandbox_id).await.ok();
    scope.commit().await.map_err(ApiError::from)?;

    // A sandbox on the Docker runtime is not on the OpenShell gateway, which is a
    // fact about where it runs and not an error: the page shows the supervisor
    // row it has and says why there is no gateway view.
    let (live, note) = match ctx.runtime.openshell_view(&sandbox.name).await {
        Ok(view) => (Some(view), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(serde_json::json!({
        "supervisor": supervisor,
        "openshell": live,
        "openshell_note": note.map(|_| "This sandbox runs on the Docker runtime, not on an OpenShell gateway, so there is no gateway view. Its own supervisor program, when its image has one, reports on the row above."),
    }))
}

#[cfg(test)]
mod status_tests {
    use super::*;
    use crate::domain::gateway::{SupervisorCheck, SupervisorReport, SupervisorStatus};

    fn check(key: &str, state: &str, detail: &str) -> SupervisorCheck {
        SupervisorCheck { key: key.into(), state: state.into(), detail: detail.into() }
    }

    fn report(checks: Vec<SupervisorCheck>, status: SupervisorStatus) -> SupervisorReport {
        SupervisorReport {
            status,
            version: None,
            config_dirtiness: 0,
            last_policy_applied_at: None,
            checks,
            facts: serde_json::Value::Null,
        }
    }

    #[test]
    fn expected_warnings_do_not_degrade_a_supervisor() {
        let r = report(
            vec![
                check("model_reachable", "warn", "no OPENAI_BASE_URL: this recipe asked for no agent"),
                check("policy_drift", "warn", "no desired policy hash delivered yet"),
                check("memory", "ok", "2066 MiB available"),
            ],
            SupervisorStatus::Degraded,
        );
        assert_eq!(effective_status(&r), SupervisorStatus::Running);
    }

    #[test]
    fn a_real_warning_still_degrades_and_a_failure_is_kept() {
        let r = report(
            vec![
                check("policy_drift", "warn", "no desired policy hash delivered yet"),
                check("memory", "warn", "219 MiB available of 7941 MiB"),
            ],
            SupervisorStatus::Degraded,
        );
        assert_eq!(effective_status(&r), SupervisorStatus::Degraded);
        let failed = report(vec![check("agent_running", "fail", "gone")], SupervisorStatus::Failed);
        assert_eq!(effective_status(&failed), SupervisorStatus::Failed);
    }
}
