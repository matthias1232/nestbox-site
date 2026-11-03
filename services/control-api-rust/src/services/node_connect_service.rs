//! Connecting a machine by its address and SSH credentials, and the state of
//! every node's tunnel (ADR 0057).
//!
//! The control plane installs a machine it has never seen: it runs the node's prepare script over
//! SSH, registers the public key the machine printed, and hands the identity back the same way.
//! The credentials a person typed are used for those two calls and never stored — not in the
//! database, not in an audit row, not in a log line.
//!
//! The tunnel is then the hub's business: the hub dials the node's WireGuard port, writes what it
//! sees to `status.tsv`, and this module turns that text into one state per node.

use std::collections::BTreeMap;

use base64::Engine;
use uuid::Uuid;

use super::{node_join_service, require, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::domain::node_tunnel::{parse_wg_dump, state_of, ConnectNodeRequest, TunnelStatus};
use crate::domain::runtime::{ExecResult, RemoteSshRun};
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode};
use crate::security::node_token;
use crate::store::TenantScope;

/// How long the prepare script may take: package installs and a Docker install on a cold machine.
const PREPARE_TIMEOUT_SECONDS: u32 = 900;

/// How long the apply script may take: images are pulled from the control plane's registry.
const APPLY_TIMEOUT_SECONDS: u32 = 1800;

/// Who asked for an install, kept for the audit trail after the request has returned.
struct Actor {
    user_id: Uuid,
    display_name: String,
    environment_id: Uuid,
}

/// The SSH login for one install.
///
/// Deliberately carries no `Debug`: a password must not be printable, so it cannot end up in a
/// log line that formats the surrounding value.
struct SshLogin {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    private_key: Option<String>,
}

/// Where an install stopped and what the machine printed there.
///
/// The output is already shortened: an audit row is a place to name the failure, not to keep a
/// package manager's whole transcript.
struct InstallFailure {
    step: &'static str,
    output: String,
}

/// Install a machine as a node over SSH. Answers immediately; the work runs in the background
/// and the node appears when its tunnel is up.
pub async fn connect(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    mut request: ConnectNodeRequest,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    if !node_join_service::enabled(config) {
        return Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "Joining nodes is not set up: NESTBOX_OVERLAY_DIR and NESTBOX_OVERLAY_ENDPOINT are empty.",
        ));
    }
    let node_name = node_join_service::valid_node_name(&request.node_name)?;
    let host = request.host.trim().to_string();
    if host.is_empty() || host.len() > 253 || host.chars().any(char::is_whitespace) {
        return Err(ApiError::validation(
            "A machine's address is a host name or IP, up to 253 characters, without spaces.",
        ));
    }
    // One credential, not two and not none: the runner picks whichever it is given, and an
    // install that carried both would be ambiguous about which one worked.
    let has_password = request.password.as_deref().is_some_and(|value| !value.trim().is_empty());
    let has_key = request.private_key.as_deref().is_some_and(|value| !value.trim().is_empty());
    if has_password == has_key {
        return Err(ApiError::validation(
            "Give either an SSH password or an SSH private key, not both and not neither.",
        ));
    }
    if !matches!(request.egress_lock.as_str(), "strict" | "exit_local") {
        return Err(ApiError::validation("A lock is 'strict' or 'exit_local'."));
    }
    if crate::store::postgres::node_gateway::name_taken(&ctx.pool, &node_name)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::conflict(
            ErrorCode::NODE_NAME_TAKEN,
            "A node with this name exists on this installation.",
        ));
    }
    // Checked here rather than in the background job: a wrong site would otherwise surface as a
    // failed install minutes later, long after the person who typed it has moved on.
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let site_known = crate::store::postgres::node_gateway::site_in_environment(&mut scope, request.site_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    if !site_known {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "site"));
    }

    request.host = host;
    let token = node_join_service::issue_token_system(
        ctx,
        config,
        principal.environment_id,
        request.site_id,
        &node_name,
        Some(principal.user_id),
    )
    .await?;
    let actor = Actor {
        user_id: principal.user_id,
        display_name: principal.display_name.clone(),
        environment_id: principal.environment_id,
    };
    let job_ctx = ctx.clone();
    let job_config = config.clone();
    let job_name = node_name.clone();
    tokio::spawn(async move {
        run(&job_ctx, &job_config, &actor, request, &job_name, &token.token).await;
    });
    Ok(serde_json::json!({ "node_name": node_name, "job": "connecting" }))
}

/// The install itself, and an honest ending either way.
///
/// Nothing here returns an error to a caller — the request was answered with 202 — so the audit
/// trail is the only place a failure is visible afterwards.
async fn run(
    ctx: &ServiceContext,
    config: &Config,
    actor: &Actor,
    mut request: ConnectNodeRequest,
    node_name: &str,
    token: &str,
) {
    let host = request.host.clone();
    let login = SshLogin {
        host: request.host.clone(),
        port: request.ssh_port,
        user: request.ssh_user.trim().to_string(),
        password: request.password.take(),
        private_key: request.private_key.take(),
    };
    let outcome = install(ctx, config, &login, &request, node_name, token).await;
    // The credentials have done their work after the second script. Dropping them here, rather
    // than at the end of the task, is what keeps the last half of this function free of them.
    drop(login);

    match outcome {
        Ok(node_id) => {
            ctx.bus.publish(LiveEvent::NodeTunnelChanged {
                node_id,
                environment_id: actor.environment_id,
                state: "unknown".to_string(),
            });
            let detail = serde_json::json!({
                "node": node_name,
                "node_id": node_id,
                "host": host,
                "lock": request.egress_lock,
            });
            if let Err(error) = audit(ctx, actor, "node.connect", Some(node_id), &detail).await {
                tracing::warn!(code = error.code.0, "the connect audit row could not be written");
            }
        }
        Err(failure) => {
            // No credentials and no token in this detail: a step name and the machine's own tail.
            let detail = serde_json::json!({
                "node": node_name,
                "host": host,
                "step": failure.step,
                "output": failure.output,
            });
            if let Err(error) = audit(ctx, actor, "node.connect_failed", None, &detail).await {
                tracing::warn!(code = error.code.0, "the failed-connect audit row could not be written");
            }
            tracing::warn!(node = node_name, step = failure.step, "connecting a machine failed");
        }
    }
}

/// Prepare the machine, register it, then apply the identity it was given.
///
/// The public key travels from the first script to the join call and back as an environment
/// variable: the machine makes its own private key, and the control plane only ever sees the
/// public half.
async fn install(
    ctx: &ServiceContext,
    config: &Config,
    login: &SshLogin,
    request: &ConnectNodeRequest,
    node_name: &str,
    token: &str,
) -> Result<Uuid, InstallFailure> {
    let prepared = must_run(
        ctx,
        login,
        "prepare",
        node_join_service::PREPARE_SCRIPT,
        BTreeMap::new(),
        PREPARE_TIMEOUT_SECONDS,
    )
    .await?;
    let public_key = public_key_from(&prepared).ok_or_else(|| InstallFailure {
        step: "prepare",
        output: tail(&prepared),
    })?;
    let answer = node_join_service::join_with_key(
        ctx,
        config,
        token,
        &public_key,
        node_name,
        Some((login.host.clone(), request.wg_listen_port)),
        &request.egress_lock,
    )
    .await
    .map_err(|error| InstallFailure {
        step: "join",
        output: tail(&error.message),
    })?;
    let env = nb_env(&answer, request.wg_listen_port, &request.egress_lock);
    must_run(
        ctx,
        login,
        "apply",
        node_join_service::APPLY_SCRIPT,
        env,
        APPLY_TIMEOUT_SECONDS,
    )
    .await?;
    Ok(answer.node_id)
}

/// Run one node script over SSH and insist that it succeeded.
///
/// The runner does the SSH; the control plane never holds an SSH client, so the credentials are
/// in one request and in nothing else.
async fn must_run(
    ctx: &ServiceContext,
    login: &SshLogin,
    step: &'static str,
    script: &str,
    env: BTreeMap<String, String>,
    timeout_seconds: u32,
) -> Result<String, InstallFailure> {
    let request = RemoteSshRun {
        host: login.host.clone(),
        port: login.port,
        user: login.user.clone(),
        password: login.password.clone(),
        private_key: login.private_key.clone(),
        script: script.to_string(),
        env,
        timeout_seconds,
    };
    let result: ExecResult = ctx
        .runtime
        .remote_ssh_run(request)
        .await
        .map_err(|error| InstallFailure {
            step,
            output: tail(&error.to_string()),
        })?;
    if result.exit_code != Some(0) {
        return Err(InstallFailure {
            step,
            output: tail(&result.output),
        });
    }
    Ok(result.output)
}

/// The variables `apply.sh` reads, and nothing else.
///
/// `NB_NODE_TOKEN` is the node's own credential: it travels over SSH to the machine that will
/// hold it, and it is never written anywhere on the control plane, which keeps only its hash.
fn nb_env(
    answer: &node_join_service::JoinAnswer,
    listen_port: u16,
    lock: &str,
) -> BTreeMap<String, String> {
    let compose = base64::engine::general_purpose::STANDARD.encode(node_join_service::compose_text());
    let mut env = BTreeMap::new();
    env.insert("NB_NODE_NAME".to_string(), answer.node_name.clone());
    env.insert("NB_OVERLAY_IP".to_string(), answer.overlay_ip.clone());
    env.insert("NB_HUB_PUBLIC_KEY".to_string(), answer.hub_public_key.clone());
    env.insert("NB_HUB_ENDPOINT".to_string(), answer.hub_endpoint.clone());
    env.insert("NB_HUB_IP".to_string(), answer.hub_ip.clone());
    env.insert("NB_REGISTRY".to_string(), answer.registry.clone());
    env.insert("NB_CONTROL_URL".to_string(), answer.control_url.clone());
    env.insert("NB_NODE_TOKEN".to_string(), answer.node_token.clone());
    env.insert("NB_NODE_ID".to_string(), answer.node_id.to_string());
    env.insert("NB_ENVIRONMENT_ID".to_string(), answer.environment_id.to_string());
    env.insert("NB_LISTEN_PORT".to_string(), listen_port.to_string());
    env.insert("NB_EGRESS_LOCK".to_string(), lock.to_string());
    env.insert("NB_COMPOSE_B64".to_string(), compose);
    env
}

/// The public key `prepare.sh` printed, as `NESTBOX_PUBKEY=<key>` on a line of its own.
fn public_key_from(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("NESTBOX_PUBKEY="))
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
}

/// The last 300 characters of what a machine said: enough to name the failure, short enough for
/// a row an operator reads in a table.
fn tail(output: &str) -> String {
    let chars: Vec<char> = output.chars().collect();
    if chars.len() <= 300 {
        output.to_string()
    } else {
        chars[chars.len() - 300..].iter().collect()
    }
}

/// Write one row about an install.
///
/// The install has no request of its own any more, so the scope is derived from the environment
/// the person was working in.
async fn audit(
    ctx: &ServiceContext,
    actor: &Actor,
    kind: &str,
    subject_id: Option<Uuid>,
    detail: &serde_json::Value,
) -> Result<(), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, actor.environment_id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(actor.user_id),
            actor_label: &actor.display_name,
            kind,
            subject_type: Some("node"),
            subject_id,
            detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// The tunnel of every node the caller can see.
pub async fn tunnels(
    ctx: &ServiceContext,
    principal: &Principal,
) -> Result<Vec<TunnelStatus>, ApiError> {
    require(principal, Permission::NodeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = crate::store::postgres::node_gateway::tunnels(&mut scope)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows)
}

/// Choose how a node reaches the internet.
pub async fn set_egress_lock(
    ctx: &ServiceContext,
    principal: &Principal,
    node_id: Uuid,
    lock: &str,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    if !matches!(lock, "strict" | "exit_local") {
        return Err(ApiError::validation("A lock is 'strict' or 'exit_local'."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::node_gateway::set_egress_lock(&mut scope, node_id, lock)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "node.egress_lock",
            subject_type: Some("node"),
            subject_id: Some(node_id),
            detail: &serde_json::json!({ "lock": lock }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({
        "node_id": node_id,
        "lock": lock,
        "note": "The node applies it within 5 minutes (sync timer)."
    }))
}

/// The lock a node should apply: read by the node itself with its token.
///
/// Authenticated exactly like the node package it also fetches (`compose_for`): the node names
/// itself in `x-nestbox-node`, presents its token as a bearer, and gets nothing but the lock.
pub async fn egress_lock_for_node(
    ctx: &ServiceContext,
    node_id: Uuid,
    presented_token: &str,
) -> Result<String, ApiError> {
    let credentials = crate::store::postgres::node_gateway::gateway_credentials(&ctx.pool, node_id)
        .await
        .map_err(ApiError::from)?;
    let Some((locator, lock)) = credentials else {
        return Err(node_token::NodeTokenError::NotConfigured.to_api());
    };
    let Some(locator) = locator else {
        return Err(node_token::NodeTokenError::NotConfigured.to_api());
    };
    node_token::verify(&locator, &ctx.secret_dir, presented_token).map_err(|error| error.to_api())?;
    Ok(lock)
}

/// Re-read the hub's WireGuard state and update the nodes.
///
/// The hub writes `wg show nb-hub dump` to `status.tsv` every three seconds; this loop is one
/// reader of that file. A missing file means the hub is not running here, which is not an error:
/// there is simply nothing to say about any tunnel.
pub async fn refresh_tunnels(ctx: &ServiceContext, config: &Config) -> Result<(), ApiError> {
    let path = config.overlay.dir.join("status.tsv");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let peers = parse_wg_dump(&text);
    if peers.is_empty() {
        return Ok(());
    }
    let nodes = crate::store::postgres::node_gateway::nodes_for_tunnel_refresh(&ctx.pool)
        .await
        .map_err(ApiError::from)?;
    let by_key: std::collections::HashMap<&str, &crate::store::postgres::node_gateway::TunnelNode> =
        nodes.iter().map(|node| (node.wg_public_key.as_str(), node)).collect();
    let now = chrono::Utc::now().timestamp();
    for peer in &peers {
        let Some(node) = by_key.get(peer.public_key.as_str()) else {
            // A peer the control plane does not know: a node removed between two refreshes.
            continue;
        };
        let Some(environment_id) = node.environment_id else {
            continue;
        };
        let (state, _age) = state_of(peer, now);
        // A peer that never handshook has no timestamp: the column means "never", not 1970.
        let handshake_at = if peer.latest_handshake == 0 {
            None
        } else {
            chrono::DateTime::from_timestamp(peer.latest_handshake, 0)
        };
        crate::store::postgres::node_gateway::record_tunnel_system(
            &ctx.pool,
            node.id,
            state,
            handshake_at,
            peer.rx_bytes,
            peer.tx_bytes,
        )
        .await
        .map_err(ApiError::from)?;
        // Only a change is an event: a tunnel that stays up would otherwise repaint every
        // console once per refresh.
        if state != node.tunnel_state {
            ctx.bus.publish(LiveEvent::NodeTunnelChanged {
                node_id: node.id,
                environment_id,
                state: state.to_string(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_key_is_read_from_the_line_prepare_prints() {
        let output = "==> Preparing the machine\nNESTBOX_PUBKEY=8GVx1N0wZ2tJ5p3m8cEoQ0Yg3ZtWk1bq6sT9eXr+pQk=\n";
        assert_eq!(
            public_key_from(output).as_deref(),
            Some("8GVx1N0wZ2tJ5p3m8cEoQ0Yg3ZtWk1bq6sT9eXr+pQk=")
        );
        assert_eq!(public_key_from("NESTBOX_PUBKEY=\n"), None);
        assert_eq!(public_key_from("nothing to see"), None);
    }

    #[test]
    fn an_audit_tail_keeps_the_end_of_the_output() {
        let short = "the machine said no";
        assert_eq!(tail(short), short);
        let long = "x".repeat(400);
        let cut = tail(&long);
        assert_eq!(cut.len(), 300);
        assert!(long.ends_with(&cut));
    }
}
