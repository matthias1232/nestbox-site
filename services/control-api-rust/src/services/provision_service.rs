//! Renting machines for a data center and handing them back (ADR 0050).
//!
//! An administrator adds a provider account (its API token goes into the vault), then asks for a
//! machine: this service checks the budget, makes a join token, has the driver create the machine
//! with the join command inside, and writes down the price and the lifetime. The machine joins by
//! itself like any other. A reaper deletes it when its lifetime is over or it has sat empty too
//! long, and never while data is still on it: a machine is only deleted once its data has a home.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{node_join_service, require, secret_service, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::domain::hosting::{self, Reaping};
use crate::error::{ApiError, ErrorCode};
use crate::hosting::{self as drivers, HostRequest, HostType};
use crate::store::TenantScope;

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

fn driver_error(error: drivers::HostError) -> ApiError {
    match error {
        drivers::HostError::Invalid(message) => ApiError::validation(message),
        drivers::HostError::Rejected(message) => ApiError::conflict(ErrorCode::NODE_RUNTIME_UNAVAILABLE, message),
        drivers::HostError::Unavailable(message) => ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, message),
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AccountView {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub max_hourly_milli: i64,
    /// What the machines of this account running now cost per hour.
    pub running_hourly_milli: i64,
    pub hosts: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HostView {
    pub id: Uuid,
    pub account: String,
    pub node_name: String,
    pub host_type: String,
    pub location: Option<String>,
    pub address: Option<String>,
    pub hourly_milli: i64,
    pub cost_so_far_milli: i64,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub idle_minutes: i32,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub destroyed_at: Option<DateTime<Utc>>,
}

async fn account_driver(ctx: &ServiceContext, environment_id: Uuid, account: Uuid) -> Result<(String, i64, Box<dyn drivers::HostDriver>), ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
    let row: Option<(String, serde_json::Value, String, i64)> =
        sqlx::query_as("SELECT kind, config, secret_name, max_hourly_milli FROM provider_accounts WHERE id = $1")
            .bind(account)
            .fetch_optional(scope.connection())
            .await
            .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    let (kind, config, secret_name, max) = row.ok_or_else(|| ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "provider account"))?;
    let secret = secret_service::reveal(ctx, environment_id, &secret_name).await?;
    let driver = drivers::driver_for(&kind, &config, &secret).map_err(driver_error)?;
    Ok((kind, max, driver))
}

/// Add a provider account. The token is sealed in the vault under `host-<name>`.
pub async fn add_account(
    ctx: &ServiceContext,
    principal: &Principal,
    kind: &str,
    name: &str,
    config: serde_json::Value,
    token: &str,
    max_hourly_milli: i64,
) -> Result<AccountView, ApiError> {
    require(principal, Permission::ProviderWrite)?;
    if !drivers::KINDS.contains(&kind) {
        return Err(ApiError::validation("A provider is hetzner, digitalocean, vultr or proxmox."));
    }
    let name = name.trim().to_lowercase();
    if crate::domain::tenancy::slugify(&name).as_deref() != Some(name.as_str()) {
        return Err(ApiError::validation("An account name uses lowercase letters, digits and dashes."));
    }
    if max_hourly_milli < 0 {
        return Err(ApiError::validation("A budget cannot be negative."));
    }
    // Build the driver once before anything is stored: a missing setting is told now, not at the
    // first machine.
    drivers::driver_for(kind, &config, token.trim()).map_err(driver_error)?;
    let secret_name = format!("host-{name}");
    secret_service::put(ctx, principal, &secret_name, "token", token).await?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO provider_accounts (environment_id, kind, name, config, secret_name, max_hourly_milli, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(kind)
    .bind(&name)
    .bind(&config)
    .bind(&secret_name)
    .bind(max_hourly_milli)
    .bind(principal.user_id)
    .fetch_one(scope.connection())
    .await
    .map_err(|e| match crate::store::map_sqlx_error(e) {
        crate::error::StoreError::UniqueViolation(_) => ApiError::conflict(ErrorCode::SANDBOX_NAME_TAKEN, "An account with this name exists."),
        other => ApiError::from(other),
    })?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(AccountView { id, kind: kind.into(), name, max_hourly_milli, running_hourly_milli: 0, hosts: 0 })
}

/// The accounts of this data center.
pub async fn accounts(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<AccountView>, ApiError> {
    require(principal, Permission::NodeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let rows: Vec<(Uuid, String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT a.id, a.kind, a.name, a.max_hourly_milli,
                coalesce((SELECT sum(h.hourly_milli) FROM provisioned_hosts h WHERE h.account_id = a.id AND h.destroyed_at IS NULL), 0)::bigint,
                (SELECT count(*) FROM provisioned_hosts h WHERE h.account_id = a.id AND h.destroyed_at IS NULL)
           FROM provider_accounts a WHERE a.environment_id = $1 ORDER BY a.name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|(id, kind, name, max_hourly_milli, running_hourly_milli, hosts)| AccountView { id, kind, name, max_hourly_milli, running_hourly_milli, hosts })
        .collect())
}

/// Remove an account that has no machine running.
pub async fn remove_account(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM provisioned_hosts WHERE account_id = $1 AND destroyed_at IS NULL")
        .bind(id)
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    if active > 0 {
        return Err(ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "Machines of this account are still running. Delete them first."));
    }
    let name: Option<String> = sqlx::query_scalar("DELETE FROM provider_accounts WHERE id = $1 AND environment_id = $2 RETURNING secret_name")
        .bind(id)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?;
    let Some(secret_name) = name else {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "provider account"));
    };
    sqlx::query("DELETE FROM stored_secrets WHERE name = $1 AND environment_id = $2")
        .bind(secret_name)
        .bind(scope.environment_id())
        .execute(scope.connection())
        .await
        .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// What an account can create, with prices.
pub async fn types(ctx: &ServiceContext, principal: &Principal, account: Uuid) -> Result<Vec<HostType>, ApiError> {
    require(principal, Permission::NodeRead)?;
    let (_, _, driver) = account_driver(ctx, principal.environment_id, account).await?;
    driver.types().await.map_err(driver_error)
}

/// Create a machine that joins this data center. Returns as soon as the provider has accepted it.
#[allow(clippy::too_many_arguments)]
pub async fn burst(
    ctx: &ServiceContext,
    config: &Config,
    principal: &Principal,
    account: Uuid,
    node_name: &str,
    site_id: Uuid,
    host_type: &str,
    location: Option<&str>,
    ttl_minutes: i64,
    idle_minutes: i32,
) -> Result<HostView, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let (kind, max, driver) = account_driver(ctx, principal.environment_id, account).await?;
    let wanted = driver
        .types()
        .await
        .map_err(driver_error)?
        .into_iter()
        .find(|t| t.name == host_type)
        .ok_or_else(|| ApiError::validation(format!("This account has no machine type '{host_type}'.")))?;
    let hourly = wanted.hourly_milli.map(|h| h as i64);
    let running: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(hourly_milli), 0)::bigint FROM provisioned_hosts WHERE account_id = $1 AND destroyed_at IS NULL",
    )
    .bind(account)
    .fetch_one(&ctx.pool)
    .await
    .map_err(db)?;
    if max > 0 && hourly.is_none() {
        return Err(ApiError::validation("This machine type has no published price, so it cannot be created under a budget."));
    }
    if !hosting::within_budget(max, running, hourly.unwrap_or(0)) {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!("That would take the account over its budget of {} per hour.", milli(max)),
        ));
    }

    let token = node_join_service::create_token(ctx, config, principal, node_name, site_id).await?;
    let request = HostRequest {
        name: token.node_name.clone(),
        host_type: host_type.to_string(),
        location: location.map(str::to_string),
        user_data: drivers::cloud_init(&token.command),
        join_command: token.command.clone(),
    };
    let created = driver.create(&request).await.map_err(driver_error)?;
    let expires_at = Utc::now() + Duration::minutes(hosting::clamp_ttl(ttl_minutes));
    let inserted: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO provisioned_hosts (environment_id, account_id, node_name, external_id, host_type, location, address,
                                        hourly_milli, state, expires_at, idle_minutes, created_by, site_id, account_name)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, (SELECT name FROM provider_accounts WHERE id = $2)) RETURNING id",
    )
    .bind(principal.environment_id)
    .bind(account)
    .bind(&token.node_name)
    .bind(&created.external_id)
    .bind(host_type)
    .bind(location)
    .bind(&created.address)
    .bind(hourly.unwrap_or(0))
    .bind(&created.state)
    .bind(expires_at)
    .bind(idle_minutes.max(0))
    .bind(principal.user_id)
    .bind(site_id)
    .fetch_one(&ctx.pool)
    .await;
    // A machine that exists at the provider but not in our books would run up a bill nobody sees:
    // if the record cannot be written, the machine goes again.
    let id = match inserted {
        Ok(id) => id,
        Err(error) => {
            let _ = driver.destroy(&created.external_id).await;
            return Err(db(error));
        }
    };
    tracing::info!(provider = %kind, node = %token.node_name, host = %created.external_id, "a machine was created for a data center");
    hosts(ctx, principal, Some(id)).await?.into_iter().next().ok_or_else(|| ApiError::internal("the new machine vanished"))
}

fn milli(value: i64) -> String {
    format!("{}.{:03}", value / 1000, value % 1000)
}

/// The machines of this data center, newest first.
pub async fn hosts(ctx: &ServiceContext, principal: &Principal, only: Option<Uuid>) -> Result<Vec<HostView>, ApiError> {
    require(principal, Permission::NodeRead)?;
    // `provider_accounts` is row-level secured, so the join needs the data center's scope.
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, String, String, String, Option<String>, Option<String>, i64, String, DateTime<Utc>, i32, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT h.id, coalesce(a.name, h.account_name, '?'), h.node_name, h.host_type, h.location, h.address, h.hourly_milli, h.state, h.expires_at,
                h.idle_minutes, h.note, h.created_at, h.destroyed_at
           FROM provisioned_hosts h LEFT JOIN provider_accounts a ON a.id = h.account_id
          WHERE h.environment_id = $1 AND ($2::uuid IS NULL OR h.id = $2)
          ORDER BY h.created_at DESC LIMIT 100",
    )
    .bind(principal.environment_id)
    .bind(only)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|(id, account, node_name, host_type, location, address, hourly_milli, state, expires_at, idle_minutes, note, created_at, destroyed_at)| HostView {
            id,
            account,
            node_name,
            host_type,
            location,
            address,
            hourly_milli,
            cost_so_far_milli: hosting::cost_so_far(hourly_milli, created_at, destroyed_at.unwrap_or(now)),
            state,
            expires_at,
            idle_minutes,
            note,
            created_at,
            destroyed_at,
        })
        .collect())
}

/// Delete a machine now. Refused while data still lives on its node.
pub async fn destroy(ctx: &ServiceContext, config: &Config, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT account_id, external_id, node_name FROM provisioned_hosts WHERE id = $1 AND environment_id = $2 AND destroyed_at IS NULL",
    )
    .bind(id)
    .bind(principal.environment_id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(db)?;
    let Some((account, external_id, node_name)) = row else {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "machine"));
    };
    delete_machine(ctx, config, principal.environment_id, id, account, &external_id, &node_name, true).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn delete_machine(
    ctx: &ServiceContext,
    config: &Config,
    environment_id: Uuid,
    host_id: Uuid,
    account: Uuid,
    external_id: &str,
    node_name: &str,
    strict: bool,
) -> Result<bool, ApiError> {
    let node: Option<Uuid> = sqlx::query_scalar("SELECT id FROM nodes WHERE name = $1 AND environment_id = $2")
        .bind(node_name)
        .bind(environment_id)
        .fetch_optional(&ctx.pool)
        .await
        .map_err(db)?;
    if let Some(node) = node {
        if !node_join_service::remove_if_empty(ctx, config, environment_id, node).await? {
            if strict {
                return Err(ApiError::conflict(
                    ErrorCode::SANDBOX_INVALID_STATE,
                    "Sandboxes, models or folders still live on this machine's node. Move them away first: a machine is only deleted once its data has a home.",
                ));
            }
            return Ok(false);
        }
    }
    let (_, _, driver) = account_driver(ctx, environment_id, account).await?;
    driver.destroy(external_id).await.map_err(driver_error)?;
    sqlx::query("UPDATE provisioned_hosts SET state = 'destroyed', destroyed_at = now() WHERE id = $1")
        .bind(host_id)
        .execute(&ctx.pool)
        .await
        .map_err(db)?;
    Ok(true)
}

/// One pass of the reaper: delete machines whose lifetime is over or that have sat empty too
/// long, never while data is on them. A machine that cannot be deleted yet gets its deadline
/// moved by half an hour and a note saying why, so the console shows it rather than hiding it.
pub async fn reap(ctx: &ServiceContext, config: &Config) -> Result<usize, ApiError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, Uuid, Uuid, String, String, DateTime<Utc>, i32, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, environment_id, account_id, external_id, node_name, expires_at, idle_minutes, idle_since
           FROM provisioned_hosts WHERE destroyed_at IS NULL AND state <> 'failed'",
    )
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    let now = Utc::now();
    help_joining(ctx, config).await;
    let mut deleted = 0;
    for (id, environment_id, account, external_id, node_name, expires_at, idle_minutes, idle_since) in rows {
        let node: Option<(Uuid, String)> = sqlx::query_as("SELECT id, status FROM nodes WHERE name = $1 AND environment_id = $2")
            .bind(&node_name)
            .bind(environment_id)
            .fetch_optional(&ctx.pool)
            .await
            .map_err(db)?;
        let (empty, online) = match &node {
            Some((node_id, status)) => (node_join_service::node_is_empty(&ctx.pool, *node_id).await?, status == "online"),
            None => (true, false),
        };
        // The idle clock runs only while the node is online and empty.
        let idle_since = match (online && empty, idle_since) {
            (true, None) => {
                sqlx::query("UPDATE provisioned_hosts SET idle_since = now(), state = 'running' WHERE id = $1").bind(id).execute(&ctx.pool).await.map_err(db)?;
                Some(now)
            }
            (true, since) => since,
            (false, Some(_)) => {
                sqlx::query("UPDATE provisioned_hosts SET idle_since = NULL WHERE id = $1").bind(id).execute(&ctx.pool).await.map_err(db)?;
                None
            }
            (false, None) => None,
        };
        if online {
            let _ = sqlx::query("UPDATE provisioned_hosts SET state = 'running' WHERE id = $1 AND state = 'creating'").bind(id).execute(&ctx.pool).await;
        }
        let verdict = hosting::reaping(now, expires_at, idle_minutes, idle_since, empty && online);
        if verdict == Reaping::Keep {
            continue;
        }
        match delete_machine(ctx, config, environment_id, id, account, &external_id, &node_name, false).await {
            Ok(true) => {
                deleted += 1;
                tracing::info!(node = %node_name, reason = ?verdict, "a rented machine was deleted");
            }
            Ok(false) => {
                sqlx::query("UPDATE provisioned_hosts SET expires_at = now() + interval '30 minutes', note = $2 WHERE id = $1")
                    .bind(id)
                    .bind("Kept past its lifetime: data is still on it. Move the sandboxes and folders away.")
                    .execute(&ctx.pool)
                    .await
                    .map_err(db)?;
            }
            Err(error) => {
                tracing::warn!(node = %node_name, code = error.code.0, "a rented machine could not be deleted yet");
                sqlx::query("UPDATE provisioned_hosts SET note = $2 WHERE id = $1")
                    .bind(id)
                    .bind(format!("Deleting failed: {}", error.message.chars().take(160).collect::<String>()))
                    .execute(&ctx.pool)
                    .await
                    .map_err(db)?;
            }
        }
    }
    Ok(deleted)
}

/// Machines that were created but have not become nodes: after a minute and a half, run the join
/// command in them again with a fresh token (up to eight tries). This is the second half of
/// creating a machine at a provider that cannot hand user data to it, and what keeps a restart of
/// the control plane from leaving a rented machine running and unjoined.
async fn help_joining(ctx: &ServiceContext, config: &Config) {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(Uuid, Uuid, Uuid, String, String, Option<Uuid>, Option<Uuid>)> = match sqlx::query_as(
        "SELECT h.id, h.environment_id, h.account_id, h.external_id, h.node_name, h.site_id, h.created_by
           FROM provisioned_hosts h
          WHERE h.destroyed_at IS NULL AND h.state = 'creating' AND h.join_attempts < 8
            AND h.created_at < now() - interval '90 seconds' AND h.site_id IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM nodes n WHERE n.name = h.node_name AND n.environment_id = h.environment_id)",
    )
    .fetch_all(&ctx.pool)
    .await
    {
        Ok(rows) => rows,
        Err(_) => return,
    };
    for (id, environment_id, account, external_id, node_name, site, created_by) in rows {
        let Some(site) = site else { continue };
        let _ = sqlx::query("UPDATE provisioned_hosts SET join_attempts = join_attempts + 1 WHERE id = $1").bind(id).execute(&ctx.pool).await;
        let Ok((_, _, driver)) = account_driver(ctx, environment_id, account).await else { continue };
        let Ok(token) = node_join_service::issue_token_system(ctx, config, environment_id, site, &node_name, created_by).await else { continue };
        match driver.run_join(&external_id, &token.command).await {
            Ok(()) => tracing::info!(node = %node_name, "the join command was started in a rented machine"),
            Err(error) => tracing::debug!(node = %node_name, error = %error, "a rented machine is not ready to join yet"),
        }
    }
}

/// The cloud-init document for a join token, for a host that is not one of the drivers.
pub async fn cloud_init_for_token(ctx: &ServiceContext, config: &Config, token: &str) -> Result<String, ApiError> {
    // `script` validates the token; the command is the same one-liner the console shows.
    node_join_service::script(ctx, config, token).await?;
    let base = if config.public_url.is_empty() { config.overlay.control_url.clone() } else { config.public_url.clone() };
    Ok(drivers::cloud_init(&format!("curl -fsSL \"{base}/api/v1/join/{token}/script\" | bash")))
}
