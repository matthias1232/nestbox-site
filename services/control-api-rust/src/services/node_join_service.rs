//! Joining a machine to a data center (ADR 0047).
//!
//! An administrator asks for a join token; the token is a one-line install on the machine. The
//! machine makes its own WireGuard key, trades the token for an identity (its overlay address,
//! its node token, the hub's key) and comes up as a node of one site of one data center.
//!
//! What this file decides: who may join (a one-time, 15-minute token bound to a data center and
//! a site), what a node is given (an address in the site's /24, a token), and what the hub is
//! told (the node's public key and address, as `peers.conf`). What it does not do: reach the
//! node (the router does), or run anything on it.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{require, secret_service, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::domain::tenancy;
use crate::error::{ApiError, ErrorCode};
use crate::runtime::router::RemoteNode;
use crate::security::{node_token, session};
use crate::store::TenantScope;

const SCRIPT: &str = include_str!("node_join/join.sh");
const COMPOSE: &str = include_str!("node_join/compose.yml");

/// The first half of an install: packages, Docker and this machine's WireGuard key. Run over SSH
/// on a machine that is not a node yet; it prints `NESTBOX_PUBKEY=<key>` (ADR 0057).
pub const PREPARE_SCRIPT: &str = include_str!("node_join/prepare.sh");

/// The second half of an install: the tunnel, the egress lock and the node package. Run over SSH
/// with the `NB_*` environment an answer from [`join_with_key`] produces (ADR 0057).
pub const APPLY_SCRIPT: &str = include_str!("node_join/apply.sh");

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

/// A join token and the line that uses it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinToken {
    pub token: String,
    pub node_name: String,
    pub expires_at: DateTime<Utc>,
    /// Paste this on the machine, as root.
    pub command: String,
}

/// What a node is given when it joins. The script reads these fields by name.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JoinAnswer {
    pub node_id: Uuid,
    pub node_name: String,
    pub environment_id: Uuid,
    pub overlay_ip: String,
    pub hub_public_key: String,
    pub hub_endpoint: String,
    pub hub_ip: String,
    pub registry: String,
    pub control_url: String,
    pub node_token: String,
    /// The egress lock the node applies once its tunnel is up: `strict` or `exit_local`.
    pub egress_lock: String,
}

/// Whether joining is set up on this installation: a hub directory and an endpoint.
pub fn enabled(config: &Config) -> bool {
    !config.overlay.dir.as_os_str().is_empty() && !config.overlay.endpoint.is_empty()
}

/// The next free host address in a site's /24. `.1` is never handed out; the hub is `10.77.0.1`
/// in a different /24, and a node's own `.1` would only confuse a reader.
pub fn next_free_host(prefix: &str, used: &[String]) -> Option<String> {
    (2..=254u32)
        .map(|n| format!("{prefix}.{n}"))
        .find(|candidate| !used.iter().any(|u| u == candidate))
}

/// The `10.77.N` prefix of a site network such as `10.77.3.0/24`.
pub fn prefix_of(cidr: &str) -> Option<String> {
    let base = cidr.split('/').next()?;
    let mut parts = base.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if a.parse::<u8>().is_ok() && b.parse::<u8>().is_ok() && c.parse::<u8>().is_ok() {
        Some(format!("{a}.{b}.{c}"))
    } else {
        None
    }
}

/// A WireGuard public key is 32 bytes in standard base64: 44 characters ending in `=`.
pub fn valid_wg_key(key: &str) -> bool {
    key.len() == 44
        && key.ends_with('=')
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
}

/// The peers file the hub applies: one `[Peer]` per node, reachable at its address only.
///
/// A node the hub dials (a machine behind NAT, ADR 0057) also carries an `Endpoint`, and then
/// the hub keeps the tunnel alive itself: a node that only listens cannot send a keepalive the
/// hub would need to answer.
pub fn render_peers(nodes: &[(String, String, String, Option<String>)]) -> String {
    let mut out = String::from("# Written by the control plane. Do not edit: the next join rewrites it.\n");
    for (name, key, ip, endpoint) in nodes {
        out.push_str(&format!("\n# {name}\n[Peer]\nPublicKey = {key}\nAllowedIPs = {ip}/32\n"));
        if let Some(endpoint) = endpoint {
            out.push_str(&format!("Endpoint = {endpoint}\nPersistentKeepalive = 25\n"));
        }
    }
    out
}

/// A node name as it is stored, or the one sentence that says why it cannot be.
///
/// One function so the token route and the "connect by address" route cannot drift apart on
/// what a node may be called.
pub fn valid_node_name(raw: &str) -> Result<String, ApiError> {
    let node_name = raw.trim().to_lowercase();
    if tenancy::slugify(&node_name).as_deref() != Some(node_name.as_str()) || node_name.len() > 42 {
        return Err(ApiError::validation(
            "A node name uses lowercase letters, digits and dashes, up to 42 characters.",
        ));
    }
    Ok(node_name)
}

/// The node package exactly as `script()` embeds it, so a base64 copy sent to a machine over SSH
/// and the heredoc in `join.sh` produce the same file.
pub fn compose_text() -> &'static str {
    COMPOSE.trim_end()
}

/// A script without its first line (the `#!` shebang).
///
/// `join.sh` is assembled inside a shell that is already running, and a second shebang in the
/// middle of a script is a comment at best and a surprise at worst.
fn without_shebang(script: &str) -> &str {
    match script.split_once('\n') {
        Some((_, rest)) => rest,
        None => script,
    }
}

/// Ask for a token that lets one machine join one site of this data center.
pub async fn create_token(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    node_name: &str,
    site_id: Uuid,
) -> Result<JoinToken, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    if !enabled(config) {
        return Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            "Joining nodes is not set up: NESTBOX_OVERLAY_DIR and NESTBOX_OVERLAY_ENDPOINT are empty.",
        ));
    }
    let node_name = valid_node_name(node_name)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let site: Option<Uuid> = sqlx::query_scalar("SELECT id FROM sites WHERE id = $1 AND environment_id = $2")
        .bind(site_id)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?;
    if site.is_none() {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "site"));
    }
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nodes WHERE name = $1)")
        .bind(&node_name)
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    if taken {
        return Err(ApiError::conflict(ErrorCode::NODE_NAME_TAKEN, "A node with this name exists on this installation."));
    }
    let token = session::random_token();
    let expires_at = Utc::now() + Duration::minutes(15);
    sqlx::query(
        "INSERT INTO node_join_tokens (token_hash, environment_id, site_id, node_name, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(session::hash_token(&token))
    .bind(scope.environment_id())
    .bind(site_id)
    .bind(&node_name)
    .bind(principal.user_id)
    .bind(expires_at)
    .execute(scope.connection())
    .await
    .map_err(db)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "node.join_token",
            subject_type: Some("site"),
            subject_id: Some(site_id),
            detail: &serde_json::json!({ "node": node_name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    let base = if config.public_url.is_empty() { config.overlay.control_url.clone() } else { config.public_url.clone() };
    Ok(JoinToken {
        command: format!("curl -fsSL \"{base}/api/v1/join/{token}/script\" | sudo bash"),
        token,
        node_name,
        expires_at,
    })
}

/// A join token made on the control plane's own initiative, for a machine it created itself.
pub async fn issue_token_system(
    ctx: &ServiceContext,
    config: &Config,
    environment_id: Uuid,
    site_id: Uuid,
    node_name: &str,
    created_by: Option<Uuid>,
) -> Result<JoinToken, ApiError> {
    let token = session::random_token();
    let expires_at = Utc::now() + Duration::minutes(15);
    sqlx::query(
        "INSERT INTO node_join_tokens (token_hash, environment_id, site_id, node_name, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(session::hash_token(&token))
    .bind(environment_id)
    .bind(site_id)
    .bind(node_name)
    .bind(created_by)
    .bind(expires_at)
    .execute(&ctx.pool)
    .await
    .map_err(db)?;
    let base = if config.public_url.is_empty() { config.overlay.control_url.clone() } else { config.public_url.clone() };
    Ok(JoinToken {
        command: format!("curl -fsSL \"{base}/api/v1/join/{token}/script\" | sudo bash"),
        token,
        node_name: node_name.to_string(),
        expires_at,
    })
}

/// The install script for a token that is still good, with the token and address filled in.
pub async fn script(ctx: &ServiceContext, config: &Config, token: &str) -> Result<String, ApiError> {
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM node_join_tokens WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now())",
    )
    .bind(session::hash_token(token))
    .fetch_one(&ctx.pool)
    .await
    .map_err(db)?;
    if !valid {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "join link"));
    }
    let base = if config.public_url.is_empty() { config.overlay.control_url.clone() } else { config.public_url.clone() };
    Ok(SCRIPT
        .replace("__CONTROL_URL__", &base)
        .replace("__TOKEN__", token)
        .replace("__COMPOSE__", compose_text())
        .replace("__PREPARE__", without_shebang(PREPARE_SCRIPT))
        .replace("__APPLY__", without_shebang(APPLY_SCRIPT)))
}

/// Trade a token for an identity. Single-use: the token is spent in the same transaction that
/// makes the node, so two machines racing on one token get one node and one refusal.
///
/// This is the node's own path: the node dials the hub, and its egress lock is the one it had
/// before the lock existed (`exit_local`).
pub async fn join(
    ctx: &ServiceContext,
    config: &Config,
    token: &str,
    public_key: &str,
    hostname: &str,
) -> Result<JoinAnswer, ApiError> {
    join_with_key(ctx, config, token, public_key, hostname, None, "exit_local").await
}

/// Trade a token for an identity, with the tunnel direction and the egress lock the caller wants.
///
/// `endpoint` is the address the hub dials when the control plane installs the machine itself
/// (`connect`): the hub then holds `Endpoint` and a keepalive for that peer. `None` keeps the
/// node-dials-hub direction, which is what a script the node runs follows.
pub async fn join_with_key(
    ctx: &ServiceContext,
    config: &Config,
    token: &str,
    public_key: &str,
    hostname: &str,
    endpoint: Option<(String, u16)>,
    lock: &str,
) -> Result<JoinAnswer, ApiError> {
    if !enabled(config) {
        return Err(ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "Joining nodes is not set up."));
    }
    if !valid_wg_key(public_key.trim()) {
        return Err(ApiError::validation("That is not a WireGuard public key."));
    }
    if !matches!(lock, "strict" | "exit_local") {
        return Err(ApiError::validation("A lock is 'strict' or 'exit_local'."));
    }
    let hub_public_key = std::fs::read_to_string(config.overlay.dir.join("hub.pub"))
        .map(|k| k.trim().to_string())
        .map_err(|_| ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "The overlay hub has not started yet."))?;

    let mut tx = ctx.pool.begin().await.map_err(db)?;
    let row: Option<(Uuid, Uuid, Uuid, String, Option<Uuid>)> = sqlx::query_as(
        "UPDATE node_join_tokens SET used_at = now()
          WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
          RETURNING id, environment_id, site_id, node_name, created_by",
    )
    .bind(session::hash_token(token))
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?;
    let Some((token_id, environment_id, site_id, node_name, created_by)) = row else {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "join link"));
    };
    sqlx::query("SELECT set_config('app.environment_id', $1, true)")
        .bind(environment_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(db)?;

    // The site's network, made on first use. Locked so two joins do not both make one.
    let (cidr,): (Option<String>,) = sqlx::query_as("SELECT overlay_cidr FROM sites WHERE id = $1 FOR UPDATE")
        .bind(site_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
    let cidr = match cidr {
        Some(cidr) => cidr,
        None => {
            let n: i64 = sqlx::query_scalar("SELECT nextval('overlay_site_seq')").fetch_one(&mut *tx).await.map_err(db)?;
            if n > 254 {
                return Err(ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "This installation has used all its site networks."));
            }
            let cidr = format!("10.77.{n}.0/24");
            sqlx::query("UPDATE sites SET overlay_cidr = $2 WHERE id = $1").bind(site_id).bind(&cidr).execute(&mut *tx).await.map_err(db)?;
            cidr
        }
    };
    let prefix = prefix_of(&cidr).ok_or_else(|| ApiError::internal("a site network is malformed"))?;
    let used: Vec<String> = sqlx::query_scalar("SELECT overlay_ip FROM nodes WHERE overlay_ip LIKE $1")
        .bind(format!("{prefix}.%"))
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
    let overlay_ip = next_free_host(&prefix, &used)
        .ok_or_else(|| ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "This site's network is full."))?;

    let node_token = session::random_token();
    let runner_ref = format!("db://node-{node_name}");
    let node_id: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, display_name, kind, status, is_local, runner_url, environment_id, site_id,
                            overlay_ip, wg_public_key, runner_token_ref, node_token_ref, joined_at, zone, region, description)
         VALUES ($1, $1, 'vm', 'unknown', false, $2, $3, $4, $5, $6, $7, $8, now(), 'overlay', $9, $10)
         RETURNING id",
    )
    .bind(&node_name)
    .bind(format!("http://{overlay_ip}:9000"))
    .bind(environment_id)
    .bind(site_id)
    .bind(&overlay_ip)
    .bind(public_key.trim())
    .bind(&runner_ref)
    .bind(node_token::hash_locator(&node_token))
    .bind(cidr.clone())
    .bind(format!("Joined from {}", hostname.chars().take(60).collect::<String>()))
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    sqlx::query("UPDATE node_join_tokens SET node_id = $2 WHERE id = $1").bind(token_id).bind(node_id).execute(&mut *tx).await.map_err(db)?;
    // How the hub reaches this node and how the node should leave: the gateway columns. Written in
    // the same transaction as the row, so a node never exists half described.
    crate::store::postgres::node_gateway::store_gateway(
        &mut tx,
        node_id,
        endpoint.as_ref().map(|(host, _port)| host.as_str()),
        endpoint.as_ref().map(|(_host, port)| i32::from(*port)),
        lock,
    )
    .await
    .map_err(ApiError::from)?;
    tx.commit().await.map_err(db)?;

    // The vault holds the token the control plane presents to this node's runner. If this
    // fails the node row exists without a way in; the join is reported as failed and the row
    // stays visible as offline for an administrator to remove, rather than being half hidden.
    secret_service::put_system(ctx, environment_id, created_by.unwrap_or_else(Uuid::nil), &format!("node-{node_name}"), "token", &node_token).await?;
    write_peers(&ctx.pool, &config.overlay.dir).await?;
    if let Some(router) = &ctx.router {
        let _ = sync(ctx, router).await;
    }
    Ok(JoinAnswer {
        node_id,
        node_name,
        environment_id,
        overlay_ip,
        hub_public_key,
        hub_endpoint: config.overlay.endpoint.clone(),
        hub_ip: config.overlay.hub_ip.clone(),
        registry: config.overlay.registry.clone(),
        control_url: config.overlay.control_url.clone(),
        node_token,
        egress_lock: lock.to_string(),
    })
}

/// The current node package, for a node that asks with its own token. This is how a node
/// follows the control plane's version: its sync timer fetches this, and if it differs from
/// what the node runs, pulls and recreates. Authenticated like every node call (ADR 0034).
pub async fn compose_for(ctx: &ServiceContext, node_id: Uuid, presented: &str) -> Result<String, ApiError> {
    let locator: Option<String> = sqlx::query_scalar("SELECT node_token_ref FROM nodes WHERE id = $1 AND wg_public_key IS NOT NULL")
        .bind(node_id)
        .fetch_optional(&ctx.pool)
        .await
        .map_err(db)?
        .flatten();
    let Some(locator) = locator else {
        return Err(node_token::NodeTokenError::NotConfigured.to_api());
    };
    node_token::verify(&locator, &ctx.secret_dir, presented).map_err(|e| e.to_api())?;
    Ok(format!("{}\n", COMPOSE.trim_end()))
}

/// Rewrite the hub's peer list from the nodes table. Atomic: written beside and renamed, so the
/// hub never reads half a file.
pub async fn write_peers(pool: &sqlx::PgPool, dir: &Path) -> Result<(), ApiError> {
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, wg_public_key, overlay_ip,
                CASE WHEN public_endpoint IS NOT NULL
                     THEN public_endpoint || ':' || wg_listen_port END
           FROM nodes
          WHERE wg_public_key IS NOT NULL AND overlay_ip IS NOT NULL ORDER BY overlay_ip",
    )
    .fetch_all(pool)
    .await
    .map_err(db)?;
    let text = render_peers(&rows);
    // Unchanged is not rewritten: the hub applies a changed file, and the loop calls this often.
    if std::fs::read_to_string(dir.join("peers.conf")).is_ok_and(|current| current == text) {
        return Ok(());
    }
    let temp = dir.join("peers.conf.tmp");
    std::fs::write(&temp, text).map_err(|e| ApiError::internal(format!("cannot write the peer list: {e}")))?;
    std::fs::rename(&temp, dir.join("peers.conf")).map_err(|e| ApiError::internal(format!("cannot replace the peer list: {e}")))?;
    Ok(())
}

/// Load the joined nodes into the router, and look at each one.
///
/// A node that answers is online; one that does not is offline, and the previous measurement
/// stays. A node never measured is measured once it answers, so its budget appears without a
/// click.
pub async fn sync(ctx: &ServiceContext, router: &crate::runtime::router::RuntimeRouter) -> Result<(), ApiError> {
    let rows: Vec<(Uuid, Uuid, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, environment_id, runner_url, runner_token_ref, status FROM nodes
          WHERE NOT is_local AND runner_url <> '' AND environment_id IS NOT NULL",
    )
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    let mut remotes = Vec::new();
    let mut environments = std::collections::HashMap::new();
    for (id, environment_id, runner_url, token_ref, _status) in &rows {
        let Some(token_ref) = token_ref else { continue };
        let Ok(token) = secret_service::credential(ctx, *environment_id, token_ref).await else {
            tracing::warn!(node = %id, "a joined node's token cannot be read from the vault");
            continue;
        };
        environments.insert(*id, *environment_id);
        remotes.push(RemoteNode { id: *id, runner_url: runner_url.clone(), token });
    }
    router.set_remotes(remotes);

    for (id, environment_id, _url, _ref, status) in rows {
        if !environments.contains_key(&id) {
            continue;
        }
        let adapter = router.for_node(Some(id));
        let answered = tokio::time::timeout(std::time::Duration::from_secs(4), adapter.health())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
        if answered {
            let measured: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM hardware_profiles WHERE node_id = $1)")
                .bind(id)
                .fetch_one(&ctx.pool)
                .await
                .unwrap_or(true);
            if status != "online" || !measured {
                if let Err(error) = super::model_service::measure_node(ctx, environment_id, id).await {
                    tracing::warn!(node = %id, code = error.code.0, "a joined node answered but could not be measured");
                }
            } else {
                sqlx::query("UPDATE nodes SET last_seen = now() WHERE id = $1").bind(id).execute(&ctx.pool).await.map_err(db)?;
            }
        } else if status != "offline" {
            sqlx::query("UPDATE nodes SET status = 'offline', updated_at = now() WHERE id = $1")
                .bind(id)
                .execute(&ctx.pool)
                .await
                .map_err(db)?;
        }
    }
    Ok(())
}

/// Whether anything still lives on a node: sandboxes, models or persistent folders.
pub async fn node_is_empty(pool: &sqlx::PgPool, node_id: Uuid) -> Result<bool, ApiError> {
    sqlx::query_scalar("SELECT nb_node_is_empty($1)")
        .bind(node_id)
        .fetch_one(pool)
        .await
        .map_err(db)
}

/// Delete an empty joined node on the control plane's own initiative (the reaper of rented
/// machines). Returns false, touching nothing, if anything still lives there.
pub async fn remove_if_empty(ctx: &ServiceContext, config: &Config, environment_id: Uuid, node_id: Uuid) -> Result<bool, ApiError> {
    if !node_is_empty(&ctx.pool, node_id).await? {
        return Ok(false);
    }
    let name: Option<String> = sqlx::query_scalar("DELETE FROM nodes WHERE id = $1 AND environment_id = $2 AND wg_public_key IS NOT NULL RETURNING name")
        .bind(node_id)
        .bind(environment_id)
        .fetch_optional(&ctx.pool)
        .await
        .map_err(db)?;
    // The peer list first: a node that is gone from the table must be gone from the hub even if
    // the clean-up below fails.
    if enabled(config) {
        write_peers(&ctx.pool, &config.overlay.dir).await?;
    }
    if let Some(name) = name {
        // `stored_secrets` is row-level secured: delete inside the data center's scope.
        let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
        sqlx::query("DELETE FROM stored_secrets WHERE name = $1")
            .bind(format!("node-{name}"))
            .execute(scope.connection())
            .await
            .map_err(db)?;
        scope.commit().await.map_err(ApiError::from)?;
    }
    if let Some(router) = &ctx.router {
        let _ = sync(ctx, router).await;
    }
    Ok(true)
}

/// Remove a joined node: its row, its token and its place on the hub. Refused while sandboxes
/// or installs still live there, because deleting the row would orphan them.
pub async fn remove(ctx: &ServiceContext, config: &Config, principal: &Principal, node_id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let (name, joined, env): (String, bool, Option<Uuid>) =
        sqlx::query_as("SELECT name, wg_public_key IS NOT NULL, environment_id FROM nodes WHERE id = $1")
            .bind(node_id)
            .fetch_optional(scope.connection())
            .await
            .map_err(db)?
            .ok_or_else(|| ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "node"))?;
    if !joined || env != Some(principal.environment_id) {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "node"));
    }
    let busy: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM sandboxes WHERE node_id = $1 AND status <> 'deleted')
              + (SELECT count(*) FROM model_installs WHERE node_id = $1)
              + (SELECT count(*) FROM persistent_roots WHERE node_id = $1 AND archived_at IS NULL)",
    )
    .bind(node_id)
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;
    if busy > 0 {
        return Err(ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "Sandboxes, models or persistent folders still live on this node. Move them to another node first: a machine is only deleted once its data has a home."));
    }
    sqlx::query("DELETE FROM nodes WHERE id = $1").bind(node_id).execute(scope.connection()).await.map_err(db)?;
    sqlx::query("DELETE FROM stored_secrets WHERE name = $1 AND environment_id = $2")
        .bind(format!("node-{name}"))
        .bind(scope.environment_id())
        .execute(scope.connection())
        .await
        .map_err(db)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "node.remove",
            subject_type: Some("node"),
            subject_id: Some(node_id),
            detail: &serde_json::json!({ "node": name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    if enabled(config) {
        write_peers(&ctx.pool, &config.overlay.dir).await?;
    }
    if let Some(router) = &ctx.router {
        let _ = sync(ctx, router).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_gets_the_first_free_address_and_never_dot_one() {
        assert_eq!(next_free_host("10.77.3", &[]), Some("10.77.3.2".into()));
        assert_eq!(next_free_host("10.77.3", &["10.77.3.2".into(), "10.77.3.3".into()]), Some("10.77.3.4".into()));
        let full: Vec<String> = (2..=254).map(|n| format!("10.77.3.{n}")).collect();
        assert_eq!(next_free_host("10.77.3", &full), None);
    }

    #[test]
    fn the_prefix_of_a_site_network_is_read_from_its_cidr() {
        assert_eq!(prefix_of("10.77.12.0/24"), Some("10.77.12".into()));
        assert_eq!(prefix_of("nonsense"), None);
    }

    #[test]
    fn only_a_wireguard_key_shaped_value_is_accepted() {
        assert!(valid_wg_key("8GVx1N0wZ2tJ5p3m8cEoQ0Yg3ZtWk1bq6sT9eXr+pQk="));
        assert!(!valid_wg_key("short"));
        assert!(!valid_wg_key(&"a".repeat(44)));
        assert!(!valid_wg_key("8GVx1N0wZ2tJ5p3m8cEoQ0Yg3ZtWk1bq6sT9eXr+p;k="));
    }

    #[test]
    fn the_peer_file_names_each_node_at_its_own_address_only() {
        let text = render_peers(&[("gpu-1".into(), "KEY1=".into(), "10.77.1.2".into(), None)]);
        assert!(text.contains("[Peer]\nPublicKey = KEY1=\nAllowedIPs = 10.77.1.2/32"));
        assert!(!text.contains("0.0.0.0"));
        // A node the hub does not dial carries no endpoint: its tunnel is the node's business.
        assert!(!text.contains("Endpoint"));
    }

    #[test]
    fn a_hub_dialled_node_carries_its_endpoint_and_a_keepalive() {
        let text = render_peers(&[(
            "far-1".into(),
            "KEY2=".into(),
            "10.77.1.3".into(),
            Some("1.2.3.4:51820".into()),
        )]);
        assert!(text.contains("Endpoint = 1.2.3.4:51820"));
        assert!(text.contains("PersistentKeepalive = 25"));
    }

    #[test]
    fn a_shebang_is_dropped_from_an_inlined_script() {
        assert_eq!(without_shebang("#!/bin/sh\nset -eu\necho hi\n"), "set -eu\necho hi\n");
        assert_eq!(without_shebang("no newline"), "no newline");
    }

    #[test]
    fn the_script_and_the_compose_file_have_the_placeholders_the_service_fills() {
        assert!(SCRIPT.contains("__CONTROL_URL__") && SCRIPT.contains("__TOKEN__") && SCRIPT.contains("__COMPOSE__"));
        assert!(!COMPOSE.contains("__"));
        // The join script must not learn the private key's value into the request it sends.
        assert!(SCRIPT.contains("\"public_key\""));
        assert!(!SCRIPT.contains("private_key"));
    }
}
