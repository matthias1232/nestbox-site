//! Users: who may sign in, with what role, in this environment.
//!
//! An admin creates and manages users; nobody else can. A user's role is their role in
//! the environment (`user_scopes`), and what they may see follows from it: an admin
//! sees every sandbox, anyone else their own and the shared ones (row-level security,
//! migration 0043). Three guards keep the environment reachable: an admin cannot demote,
//! disable or remove themselves, and the last admin cannot go.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal, Role};
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::security::password;
use crate::store::TenantScope;

const MIN_PASSWORD: usize = 12;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserView {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// The role in this environment.
    pub role: String,
    /// `active` or `disabled`.
    pub status: String,
    /// Sandboxes this user owns here.
    pub sandboxes: i64,
    pub is_you: bool,
}

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(StoreError::Database(error))
}

/// The organisation that owns the environment the caller works in. Not the caller's own: a
/// provider's administrator acting inside a customer's data center creates users of the
/// customer (ADR 0044).
async fn organization_of(ctx: &ServiceContext, environment_id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar("SELECT organization_id FROM environments WHERE id = $1")
        .bind(environment_id)
        .fetch_one(&ctx.pool)
        .await
        .map_err(db)
}

fn check_email(email: &str) -> Result<String, ApiError> {
    let email = email.trim().to_lowercase();
    let valid = email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        })
        && !email.contains(char::is_whitespace);
    if valid { Ok(email) } else { Err(ApiError::validation("Enter an email address, for example name@example.com.")) }
}

fn check_password(value: &str) -> Result<(), ApiError> {
    if value.chars().count() < MIN_PASSWORD {
        return Err(ApiError::validation(format!("A password has at least {MIN_PASSWORD} characters.")));
    }
    Ok(())
}

fn parse_role(value: &str) -> Result<Role, ApiError> {
    Role::parse(value).ok_or_else(|| ApiError::validation("A role is admin, operator, member or viewer."))
}

async fn admin_count(scope: &mut TenantScope<'_>, excluding: Option<Uuid>) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM user_scopes s JOIN users u ON u.id = s.user_id
          WHERE s.environment_id = $1 AND s.role = 'admin' AND u.status = 'active'
            AND ($2::uuid IS NULL OR s.user_id <> $2)",
    )
    .bind(scope.environment_id())
    .bind(excluding)
    .fetch_one(scope.connection())
    .await
    .map_err(db)
}

async fn audit(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    kind: &str,
    subject: Uuid,
    detail: serde_json::Value,
) -> Result<(), ApiError> {
    crate::store::postgres::audit::record(
        scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind,
            subject_type: Some("user"),
            subject_id: Some(subject),
            detail: &detail,
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)
}

pub async fn list(ctx: &ServiceContext, principal: &Principal) -> Result<Vec<UserView>, ApiError> {
    require(principal, Permission::UserManage)?;
    let org = organization_of(ctx, principal.environment_id).await?;
    // Inside the tenant scope: the sandbox count reads a table under row-level security.
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let rows: Vec<(Uuid, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT u.id, u.email, u.display_name, s.role, u.status,
                (SELECT count(*) FROM sandboxes b WHERE b.owner_user_id = u.id AND b.environment_id = $2)
           FROM users u JOIN user_scopes s ON s.user_id = u.id AND s.environment_id = $2
          WHERE u.organization_id = $1 ORDER BY u.display_name, u.email",
    )
    .bind(org)
    .bind(principal.environment_id)
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows
        .into_iter()
        .map(|(id, email, display_name, role, status, sandboxes)| UserView {
            is_you: id == principal.user_id,
            id,
            email,
            display_name,
            role,
            status,
            sandboxes,
        })
        .collect())
}

pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    email: &str,
    display_name: &str,
    password_value: &str,
    role: &str,
) -> Result<UserView, ApiError> {
    require(principal, Permission::UserManage)?;
    let email = check_email(email)?;
    let display_name = display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 80 {
        return Err(ApiError::validation("A name is 1 to 80 characters."));
    }
    check_password(password_value)?;
    let role = parse_role(role)?;
    let org = organization_of(ctx, principal.environment_id).await?;
    let hash = password::hash_password(password_value.to_string())
        .await
        .map_err(|_| ApiError::validation("The password could not be processed."))?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE organization_id = $1 AND email = $2)")
        .bind(org)
        .bind(&email)
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    if exists {
        return Err(ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, "A user with this email already exists."));
    }
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, email, display_name, password_hash, role)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(org)
    .bind(&email)
    .bind(display_name)
    .bind(&hash)
    .bind(role.as_str())
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;
    sqlx::query("INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(principal.environment_id)
        .bind(role.as_str())
        .execute(scope.connection())
        .await
        .map_err(db)?;
    audit(&mut scope, principal, "user.create", id, serde_json::json!({ "email": email, "role": role.as_str() })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(UserView {
        id,
        email,
        display_name: display_name.to_string(),
        role: role.as_str().to_string(),
        status: "active".into(),
        sandboxes: 0,
        is_you: false,
    })
}

/// Change a user's name, role or status.
pub async fn update(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    display_name: Option<&str>,
    role: Option<&str>,
    status: Option<&str>,
) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    let org = organization_of(ctx, principal.environment_id).await?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let current: Option<(String, String)> = sqlx::query_as(
        "SELECT s.role, u.status FROM users u JOIN user_scopes s ON s.user_id = u.id AND s.environment_id = $2
          WHERE u.id = $1 AND u.organization_id = $3",
    )
    .bind(id)
    .bind(principal.environment_id)
    .bind(org)
    .fetch_optional(scope.connection())
    .await
    .map_err(db)?;
    let Some((current_role, current_status)) = current else {
        return Err(ApiError::from(StoreError::NotFound));
    };
    let new_role = match role {
        Some(value) => parse_role(value)?.as_str().to_string(),
        None => current_role.clone(),
    };
    let new_status = match status {
        Some("active") => "active",
        Some("disabled") => "disabled",
        Some(_) => return Err(ApiError::validation("A status is active or disabled.")),
        None => current_status.as_str(),
    }
    .to_string();
    let loses_admin = current_role == "admin" && current_status == "active" && (new_role != "admin" || new_status != "active");
    if loses_admin {
        if id == principal.user_id {
            return Err(ApiError::validation("You cannot demote or disable yourself: ask another admin."));
        }
        if admin_count(&mut scope, Some(id)).await? == 0 {
            return Err(ApiError::validation("This is the last admin; the environment would have none."));
        }
    }
    if let Some(name) = display_name {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(ApiError::validation("A name is 1 to 80 characters."));
        }
        sqlx::query("UPDATE users SET display_name = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(name)
            .execute(scope.connection())
            .await
            .map_err(db)?;
    }
    sqlx::query("UPDATE user_scopes SET role = $3 WHERE user_id = $1 AND environment_id = $2")
        .bind(id)
        .bind(principal.environment_id)
        .bind(&new_role)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    sqlx::query("UPDATE users SET status = $2, role = $3, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(&new_status)
        .bind(&new_role)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    if new_status == "disabled" || new_role != current_role {
        // A signed-in session keeps the role it was made with: end it, so a change
        // takes hold at once instead of at the next login.
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(id)
            .execute(scope.connection())
            .await
            .map_err(db)?;
    }
    audit(
        &mut scope,
        principal,
        "user.update",
        id,
        serde_json::json!({ "role": new_role, "status": new_status, "renamed": display_name.is_some() }),
    )
    .await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Set a new password and end the user's sessions.
pub async fn set_password(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    password_value: &str,
) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    check_password(password_value)?;
    let org = organization_of(ctx, principal.environment_id).await?;
    let hash = password::hash_password(password_value.to_string())
        .await
        .map_err(|_| ApiError::validation("The password could not be processed."))?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let affected = sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1 AND organization_id = $3")
        .bind(id)
        .bind(&hash)
        .bind(org)
        .execute(scope.connection())
        .await
        .map_err(db)?
        .rows_affected();
    if affected == 0 {
        return Err(ApiError::from(StoreError::NotFound));
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    audit(&mut scope, principal, "user.password", id, serde_json::json!({})).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Take a user out of this environment. Their account stays; their sandboxes stay too,
/// owned by nobody, and an admin sees them.
pub async fn remove(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::UserManage)?;
    if id == principal.user_id {
        return Err(ApiError::validation("You cannot remove yourself: ask another admin."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let role: Option<String> = sqlx::query_scalar("SELECT role FROM user_scopes WHERE user_id = $1 AND environment_id = $2")
        .bind(id)
        .bind(principal.environment_id)
        .fetch_optional(scope.connection())
        .await
        .map_err(db)?;
    let Some(role) = role else {
        return Err(ApiError::from(StoreError::NotFound));
    };
    if role == "admin" && admin_count(&mut scope, Some(id)).await? == 0 {
        return Err(ApiError::validation("This is the last admin; the environment would have none."));
    }
    sqlx::query("UPDATE sandboxes SET owner_user_id = NULL WHERE owner_user_id = $1 AND environment_id = $2")
        .bind(id)
        .bind(principal.environment_id)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    sqlx::query("DELETE FROM user_scopes WHERE user_id = $1 AND environment_id = $2")
        .bind(id)
        .bind(principal.environment_id)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(scope.connection())
        .await
        .map_err(db)?;
    audit(&mut scope, principal, "user.remove", id, serde_json::json!({ "role": role })).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}
