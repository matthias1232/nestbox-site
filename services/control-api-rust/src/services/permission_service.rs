//! Permissions per scope: who has which role in a data center, a sandbox or an organisation
//! (contract C13). It reads and writes the shipped model (`user_scopes`, `sandbox_grants`,
//! the organisation tree) and invents nothing.

use uuid::Uuid;

use super::{require, tenancy_service, ServiceContext};
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::permissions::{allowed_role_for, ScopeType};
use crate::error::{ApiError, ErrorCode};
use crate::store::postgres::permissions as store;
use crate::store::TenantScope;

fn not_found() -> ApiError {
    ApiError::not_found(ErrorCode::NOT_FOUND, "No such scope.")
}

/// Open a scope on a data center the caller administers. A data center the caller does not
/// reach is a 404; one they reach without being admin there is a 403.
async fn open_environment<'a>(
    ctx: &'a ServiceContext,
    principal: &Principal,
    environment_id: Uuid,
) -> Result<TenantScope<'a>, ApiError> {
    if environment_id == principal.environment_id {
        return TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from);
    }
    let reach = crate::store::postgres::auth::environments_for_user(&ctx.pool, principal.user_id)
        .await
        .map_err(ApiError::from)?;
    let role = reach
        .iter()
        .find(|(id, _, _)| *id == environment_id)
        .map(|(_, _, role)| *role)
        .ok_or_else(not_found)?;
    if !role.includes(Permission::UserManage.required_role()) {
        return Err(ApiError::forbidden(Permission::UserManage.code()));
    }
    TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)
}

/// Whether the sandbox is visible to the caller in their data center.
async fn sandbox_visible(scope: &mut TenantScope<'_>, id: Uuid) -> Result<bool, ApiError> {
    let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM sandboxes WHERE id = $1 AND environment_id = $2")
        .bind(id)
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    Ok(found.is_some())
}

/// Who has which role in a scope, plus open invitations (contract C13).
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    scope_type: ScopeType,
    scope_id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::UserManage)?;
    match scope_type {
        ScopeType::Organization => {
            let reachable = store::organization_reachable(&ctx.pool, principal.user_id, scope_id)
                .await
                .map_err(ApiError::from)?;
            if !reachable {
                return Err(not_found());
            }
            let items = store::list_organization(&ctx.pool, scope_id).await.map_err(ApiError::from)?;
            Ok(serde_json::json!({ "items": items, "invitations": [] }))
        }
        ScopeType::Environment => {
            let mut scope = open_environment(ctx, principal, scope_id).await?;
            let items = store::list_environment(&mut scope).await.map_err(ApiError::from)?;
            let invitations = store::list_invitations(&mut scope, None).await.map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            Ok(serde_json::json!({ "items": items, "invitations": invitations }))
        }
        ScopeType::Sandbox => {
            let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
            if !sandbox_visible(&mut scope, scope_id).await? {
                return Err(not_found());
            }
            let items = store::list_sandbox(&mut scope, scope_id).await.map_err(ApiError::from)?;
            let invitations = store::list_invitations(&mut scope, Some(scope_id)).await.map_err(ApiError::from)?;
            scope.commit().await.map_err(ApiError::from)?;
            Ok(serde_json::json!({ "items": items, "invitations": invitations }))
        }
    }
}

async fn audit(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    kind: &str,
    environment_id: Uuid,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    crate::store::postgres::audit::record(
        scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some("environment"),
            subject_id: Some(environment_id),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

fn last_admin() -> ApiError {
    ApiError::conflict(
        ErrorCode("permission.last_admin"),
        "A data center needs at least one administrator.",
    )
}

/// Give a user a role in a scope. A data center takes any role; a sandbox takes `member` only
/// and goes through the existing grant; the organisation tree cannot be written here.
pub async fn set(
    ctx: &ServiceContext,
    principal: &Principal,
    user_id: Uuid,
    scope_type: ScopeType,
    scope_id: Uuid,
    role: &str,
) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    let parsed = Role::parse(role).ok_or_else(|| ApiError::validation("Unknown role."))?;
    allowed_role_for(scope_type, parsed).map_err(ApiError::validation)?;
    match scope_type {
        ScopeType::Organization => Err(ApiError::validation("The organisation tree is read-only here.")),
        ScopeType::Sandbox => tenancy_service::grant(ctx, principal, scope_id, user_id).await,
        ScopeType::Environment => {
            let mut scope = open_environment(ctx, principal, scope_id).await?;
            if !store::user_may_join(&mut scope, user_id).await.map_err(ApiError::from)? {
                return Err(ApiError::validation("This person does not belong to this organisation."));
            }
            let current = store::direct_role(&mut scope, user_id).await.map_err(ApiError::from)?;
            if current.as_deref() == Some("admin")
                && parsed != Role::Admin
                && store::other_direct_admins(&mut scope, user_id).await.map_err(ApiError::from)? == 0
            {
                return Err(last_admin());
            }
            store::set_scope(&mut scope, user_id, parsed.as_str()).await.map_err(ApiError::from)?;
            audit(
                &mut scope,
                principal,
                "permission.set",
                scope_id,
                serde_json::json!({ "user_id": user_id, "role": parsed.as_str(), "previous": current }),
            )
            .await?;
            scope.commit().await.map_err(ApiError::from)?;
            Ok(())
        }
    }
}

/// Take a user's role in a scope away. The last administrator of a data center cannot be removed.
pub async fn remove(
    ctx: &ServiceContext,
    principal: &Principal,
    user_id: Uuid,
    scope_type: ScopeType,
    scope_id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    match scope_type {
        ScopeType::Organization => Err(ApiError::validation("The organisation tree is read-only here.")),
        ScopeType::Sandbox => tenancy_service::revoke(ctx, principal, scope_id, user_id).await,
        ScopeType::Environment => {
            let mut scope = open_environment(ctx, principal, scope_id).await?;
            let current = store::direct_role(&mut scope, user_id).await.map_err(ApiError::from)?;
            if current.as_deref() == Some("admin")
                && store::other_direct_admins(&mut scope, user_id).await.map_err(ApiError::from)? == 0
            {
                return Err(last_admin());
            }
            store::remove_scope(&mut scope, user_id).await.map_err(ApiError::from)?;
            audit(
                &mut scope,
                principal,
                "permission.remove",
                scope_id,
                serde_json::json!({ "user_id": user_id, "previous": current }),
            )
            .await?;
            scope.commit().await.map_err(ApiError::from)?;
            Ok(())
        }
    }
}
