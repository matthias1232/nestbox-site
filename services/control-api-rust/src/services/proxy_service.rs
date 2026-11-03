//! The exit proxy catalogue, and choosing one.
//!
//! Two questions are answered here: which exits exist, and which of them a
//! sandbox or a provider profile leaves through. The catalogue is data, so an
//! operator adds the firewall's exits once and every selector is populated from
//! the same rows.
//!
//! Choosing an exit for a *running* sandbox changes the environment its tools
//! read (`HTTP_PROXY`), which Docker fixes at container creation. The change is
//! therefore applied by recreating the container, exactly like a mount change,
//! and the response says so instead of pretending a live switch happened.

use uuid::Uuid;

use crate::domain::proxy::{NewProxy, Proxy, SandboxProxy};
use crate::error::{ApiError, ErrorCode};
use crate::domain::auth::{Permission, Principal};
use crate::services::access_control::require;
use crate::services::{sandbox_service, ServiceContext};
use crate::store::postgres as store;
use crate::store::TenantScope;

/// List every exit in this environment.
pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<Proxy>, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let items = store::proxies::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(items)
}

/// One exit.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<Proxy, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let proxy = store::proxies::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(proxy)
}

/// Validate what an operator typed into the form.
///
/// The check happens here rather than in the database because the message is
/// what the operator reads, and a constraint violation is not a sentence.
fn validate(new: &NewProxy) -> Result<NewProxy, ApiError> {
    let name = new.name.trim();
    if name.is_empty() {
        return Err(ApiError::validation("The exit needs a name."));
    }
    let host = new.host.trim();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err(ApiError::validation(
            "The exit needs a host without spaces (an address or a name).",
        ));
    }
    if !(1..=65535).contains(&new.port) {
        return Err(ApiError::validation("The port must be between 1 and 65535."));
    }
    Ok(NewProxy {
        name: name.to_string(),
        kind: new.kind,
        host: host.to_string(),
        port: new.port,
        country: new.country.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
        provider: new.provider.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
        username: new.username.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
        password_ref: new.password_ref.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
    })
}

/// Add an exit to the catalogue.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    new: NewProxy,
) -> Result<Proxy, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let new = validate(&new)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let proxy = match store::proxies::insert(&mut scope, &new).await {
        Ok(proxy) => proxy,
        Err(crate::error::StoreError::Database(error)) if is_unique_violation(&error) => {
            return Err(ApiError::conflict(
                ErrorCode::CONFLICT,
                format!("An exit named '{}' already exists.", new.name),
            ));
        }
        Err(other) => return Err(ApiError::from(other)),
    };
    store::audit::record(
        &mut scope,
        store::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "proxy.create",
            subject_type: Some("proxy"),
            subject_id: Some(proxy.id),
            detail: &serde_json::json!({ "name": proxy.name, "host": proxy.host, "port": proxy.port }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(proxy)
}

/// Change an exit.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    version: i64,
    new: NewProxy,
    enabled: bool,
) -> Result<Proxy, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let new = validate(&new)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let proxy = match store::proxies::update(&mut scope, id, version, &new, enabled).await {
        Ok(proxy) => proxy,
        Err(crate::error::StoreError::Database(error)) if is_unique_violation(&error) => {
            return Err(ApiError::conflict(
                ErrorCode::CONFLICT,
                format!("An exit named '{}' already exists.", new.name),
            ));
        }
        Err(other) => return Err(ApiError::from(other)),
    };
    scope.commit().await.map_err(ApiError::from)?;
    Ok(proxy)
}

/// Remove an exit. Anything that pointed at it falls back to the node's route.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    store::proxies::delete(&mut scope, id).await.map_err(ApiError::from)?;
    store::audit::record(
        &mut scope,
        store::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "proxy.delete",
            subject_type: Some("proxy"),
            subject_id: Some(id),
            detail: &serde_json::json!({}),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|db| db.code())
        .is_some_and(|code| code == "23505")
}

/// The exit a sandbox uses, if any.
pub async fn for_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<SandboxProxy, ApiError> {
    require(principal, Permission::PolicyRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // `get` first so a sandbox from another tenant is a 404 rather than "no exit".
    store::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let proxy = store::proxies::for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(SandboxProxy { sandbox_id, proxy })
}

/// Choose the exit a sandbox leaves through.
///
/// Returns the sandbox and whether its container was recreated, so the console
/// can say "applied" rather than "saved, maybe later". Recreating is what makes
/// the choice real: the proxy variables a tool reads are fixed when the
/// container is created.
pub async fn set_for_sandbox(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    proxy_id: Option<Uuid>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::sandboxes::get(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    if let Some(id) = proxy_id {
        // Validated through the same scope: another tenant's exit is a 404.
        let proxy = store::proxies::get(&mut scope, id).await.map_err(ApiError::from)?;
        if !proxy.enabled {
            return Err(ApiError::validation(
                "That exit is disabled; enable it or choose another.",
            ));
        }
    }
    store::proxies::set_sandbox_proxy(&mut scope, sandbox_id, proxy_id)
        .await
        .map_err(ApiError::from)?;
    store::audit::record(
        &mut scope,
        store::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.proxy",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &serde_json::json!({ "proxy_id": proxy_id }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut applied = false;
    if sandbox.status == crate::domain::sandbox::SandboxStatus::Running {
        // Best effort and reported: a sandbox whose container is gone is
        // reconciled by the next start, and the choice is already stored.
        if let Ok(result) = sandbox_service::recreate(ctx, principal, sandbox_id).await {
            applied = result.get("recreated").and_then(|v| v.as_bool()).unwrap_or(false);
        }
    }
    Ok(serde_json::json!({ "proxy_id": proxy_id, "applied": applied }))
}

/// Choose the exit the control plane uses when it calls a provider.
pub async fn set_for_provider(
    ctx: &ServiceContext,
    principal: &Principal,
    provider_id: Uuid,
    proxy_id: Option<Uuid>,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    store::providers::get(&mut scope, provider_id)
        .await
        .map_err(ApiError::from)?;
    if let Some(id) = proxy_id {
        store::proxies::get(&mut scope, id).await.map_err(ApiError::from)?;
    }
    store::proxies::set_provider_proxy(&mut scope, provider_id, proxy_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({ "provider_id": provider_id, "proxy_id": proxy_id }))
}

/// The URL of the exit a provider is called through, if one is chosen.
///
/// Used where the platform makes an outbound request to a provider: the choice
/// is a row, and this is the only place it turns into a URL.
pub async fn provider_proxy_url(
    ctx: &ServiceContext,
    principal: &Principal,
    provider_id: Uuid,
) -> Result<Option<String>, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let proxy = store::proxies::for_provider(&mut scope, provider_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(proxy.filter(|p| p.enabled).map(|p| p.url()))
}
