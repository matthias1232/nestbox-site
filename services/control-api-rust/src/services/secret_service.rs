//! The secret store: keys typed into the console, kept encrypted, never shown again.
//!
//! Write-only by design. The API accepts a value once and answers with its last four
//! characters; the only reader of the clear value is code inside this process that
//! needs to call a provider on an operator's behalf.

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::security::vault;
use crate::store::postgres::vault as store;
use crate::store::postgres::vault::SecretMeta;
use crate::store::TenantScope;

const KINDS: [&str; 3] = ["api_key", "token", "password"];

fn key(ctx: &ServiceContext) -> Result<[u8; 32], ApiError> {
    vault::master_key(&ctx.secret_dir).map_err(|error| {
        ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::SECRET_UNRESOLVABLE,
            error.message(),
        )
    })
}

pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<SecretMeta>, ApiError> {
    // Key names and endings are for whoever manages the keys.
    require(principal, Permission::ProviderWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let items = store::list(&mut scope).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(items)
}

/// Store a value under a name; an existing name is rotated, not duplicated.
pub async fn put(
    ctx: &ServiceContext,
    principal: &Principal,
    name: &str,
    kind: &str,
    value: &str,
) -> Result<SecretMeta, ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_'))
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric();
    if !valid {
        return Err(ApiError::validation(
            "A secret name uses lowercase letters, digits, dots, dashes and underscores, and starts and ends with a letter or digit.",
        ));
    }
    if !KINDS.contains(&kind) {
        return Err(ApiError::validation("A secret kind is one of: api_key, token, password."));
    }
    let value = value.trim();
    if value.len() < 8 || value.len() > 8192 || value.contains('\n') {
        return Err(ApiError::validation("A secret value is one line of 8 to 8192 characters."));
    }
    let master = key(ctx)?;
    let (ciphertext, nonce) = vault::seal(&master, &principal.environment_id.to_string(), name, value)
        .map_err(|error| ApiError::validation(error.message()))?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let meta = store::upsert(&mut scope, name, kind, &ciphertext, &nonce, &vault::last4(value), principal.user_id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "secret.put",
            subject_type: Some("secret"),
            subject_id: Some(meta.id),
            // The name and kind only; the value is never written anywhere but the vault.
            detail: &serde_json::json!({ "name": name, "kind": kind }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(meta)
}

/// Store a value on the control plane's own initiative (a joined node's token). Same sealing and
/// the same table as `put`; the difference is that no caller is asked for a permission, because
/// the caller is the join flow that already proved a one-time token.
pub async fn put_system(
    ctx: &ServiceContext,
    environment_id: Uuid,
    created_by: Uuid,
    name: &str,
    kind: &str,
    value: &str,
) -> Result<(), ApiError> {
    let master = key(ctx)?;
    let (ciphertext, nonce) = vault::seal(&master, &environment_id.to_string(), name, value)
        .map_err(|error| ApiError::validation(error.message()))?;
    let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
    store::upsert(&mut scope, name, kind, &ciphertext, &nonce, &vault::last4(value), created_by)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Delete a secret. Refused while a provider still points at it, so a key is not
/// removed out from under a working sandbox.
pub async fn delete(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::ProviderWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let all = store::list(&mut scope).await.map_err(ApiError::from)?;
    let Some(item) = all.into_iter().find(|item| item.id == id) else {
        return Err(ApiError::from(StoreError::NotFound));
    };
    let users = store::providers_using(&mut scope, &item.name).await.map_err(ApiError::from)?;
    if !users.is_empty() {
        return Err(ApiError::conflict(
            ErrorCode::SECRET_UNRESOLVABLE,
            format!("Providers still use this key: {}. Point them elsewhere first.", users.join(", ")),
        ));
    }
    store::delete(&mut scope, id).await.map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "secret.delete",
            subject_type: Some("secret"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "name": item.name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// The clear value behind a `db://<name>` locator, for the code that calls a provider.
pub async fn reveal(ctx: &ServiceContext, environment_id: Uuid, name: &str) -> Result<String, ApiError> {
    let master = key(ctx)?;
    let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
    let (ciphertext, nonce) = store::sealed(&mut scope, name).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    vault::open(&master, &environment_id.to_string(), name, &ciphertext, &nonce).map_err(|error| {
        ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::SECRET_UNRESOLVABLE, error.message())
    })
}

/// Resolve any credential locator: `db://<name>` from the vault, `file://` from disk.
pub async fn credential(
    ctx: &ServiceContext,
    environment_id: Uuid,
    locator: &str,
) -> Result<String, String> {
    if let Some(name) = locator.strip_prefix("db://") {
        return reveal(ctx, environment_id, name).await.map_err(|error| error.message);
    }
    crate::security::secrets::resolve(locator, &ctx.secret_dir)
        .map_err(|error| error.message().to_string())
}
