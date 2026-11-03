//! Users, sessions, API tokens, and environment membership.
//!
//! Sessions are looked up by token hash, never by the token, so the plaintext
//! exists only in the browser's cookie.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::auth::Role;
use crate::error::StoreError;

/// A user row needed to authenticate.
pub struct UserRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub role: Role,
    pub status: String,
}

/// Find a user by email, without a tenant scope.
///
/// Login happens before a tenant is known, so this reads from the pool rather
/// than a scoped transaction. The users table is intentionally not
/// tenant-scoped: a user belongs to an organization, and the environment is
/// chosen after authentication.
pub async fn find_user_by_email(
    pool: &sqlx::PgPool,
    email: &str,
) -> Result<Option<UserRecord>, StoreError> {
    let row = sqlx::query(
        "SELECT id, organization_id, email, display_name, password_hash, role, status
           FROM users WHERE email = $1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    let role_raw: String = row.try_get("role")?;
    Ok(Some(UserRecord {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        password_hash: row.try_get("password_hash")?,
        role: Role::parse(&role_raw).unwrap_or(Role::Viewer),
        status: row.try_get("status")?,
    }))
}

/// The environments a user may access, with the role in each.
pub async fn environments_for_user(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Vec<(Uuid, String, Role)>, StoreError> {
    let rows = sqlx::query(
        "SELECT e.id, e.name, s.role
           FROM nb_user_environments($1) s
           JOIN environments e ON e.id = s.environment_id
          ORDER BY e.name",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let role_raw: String = row.try_get("role")?;
        out.push((
            row.try_get("id")?,
            row.try_get("name")?,
            Role::parse(&role_raw).unwrap_or(Role::Viewer),
        ));
    }
    Ok(out)
}

/// Store a new session.
pub async fn insert_session(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    token_hash: &str,
    csrf_hash: &str,
    expires_at: DateTime<Utc>,
    idle_deadline: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, csrf_hash, expires_at, idle_deadline)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(csrf_hash)
    .bind(expires_at)
    .bind(idle_deadline)
    .execute(pool)
    .await?;
    Ok(())
}

/// A session as looked up on a request.
pub struct SessionRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub csrf_hash: String,
    pub expires_at: DateTime<Utc>,
    pub idle_deadline: DateTime<Utc>,
    pub display_name: String,
    pub status: String,
}

/// Resolve a session by its token hash, rejecting an expired one.
pub async fn find_session(
    pool: &sqlx::PgPool,
    token_hash: &str,
) -> Result<Option<SessionRecord>, StoreError> {
    let row = sqlx::query(
        "SELECT s.id, s.user_id, s.csrf_hash, s.expires_at, s.idle_deadline,
                u.display_name, u.status
           FROM sessions s
           JOIN users u ON u.id = s.user_id
          WHERE s.token_hash = $1
            AND s.expires_at > now()
            AND s.idle_deadline > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else { return Ok(None) };
    Ok(Some(SessionRecord {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        csrf_hash: row.try_get("csrf_hash")?,
        expires_at: row.try_get("expires_at")?,
        idle_deadline: row.try_get("idle_deadline")?,
        display_name: row.try_get("display_name")?,
        status: row.try_get("status")?,
    }))
}

/// Extend a session's idle deadline after a request.
pub async fn touch_session(
    pool: &sqlx::PgPool,
    session_id: Uuid,
    idle_deadline: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query("UPDATE sessions SET last_seen_at = now(), idle_deadline = $2 WHERE id = $1")
        .bind(session_id)
        .bind(idle_deadline)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replace a session's CSRF hash. The token itself is never stored, so a client that
/// lost it (a page reload) can only be given a new one.
pub async fn rotate_csrf(
    pool: &sqlx::PgPool,
    token_hash: &str,
    csrf_hash: &str,
) -> Result<bool, StoreError> {
    let done = sqlx::query(
        "UPDATE sessions SET csrf_hash = $2
          WHERE token_hash = $1 AND expires_at > now() AND idle_deadline > now()",
    )
    .bind(token_hash)
    .bind(csrf_hash)
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Revoke a session. Deletion, not a flag: a revoked row cannot be resurrected
/// by a bug that forgets to check the flag.
pub async fn delete_session(pool: &sqlx::PgPool, token_hash: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// The role a user holds in a specific environment.
pub async fn role_in_environment(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    environment_id: Uuid,
) -> Result<Option<Role>, StoreError> {
    let row = sqlx::query(
        "SELECT role FROM nb_user_environments($1) WHERE environment_id = $2",
    )
    .bind(user_id)
    .bind(environment_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|row| {
        row.try_get::<String, _>("role")
            .ok()
            .and_then(|raw| Role::parse(&raw))
    }))
}

/// Replace or set the password hash of the bootstrap administrator.
///
/// Used once, on first start, to replace the `!bootstrap` sentinel with a real
/// hash. Returns whether a row was changed.
pub async fn set_password_if_bootstrap(
    pool: &sqlx::PgPool,
    email: &str,
    hash: &str,
) -> Result<bool, StoreError> {
    let affected = sqlx::query(
        "UPDATE users SET password_hash = $2, updated_at = now()
          WHERE email = $1 AND password_hash = '!bootstrap'",
    )
    .bind(email)
    .bind(hash)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Create an API token for automation.
pub async fn insert_api_token(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    name: &str,
    token_hash: &str,
    expires_at: Option<DateTime<Utc>>,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO api_tokens (user_id, name, token_hash, expires_at)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Resolve an API token to its user.
pub async fn find_api_token_user(
    pool: &sqlx::PgPool,
    token_hash: &str,
) -> Result<Option<Uuid>, StoreError> {
    let row = sqlx::query(
        "UPDATE api_tokens SET last_used_at = now()
          WHERE token_hash = $1 AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at > now())
          RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| row.try_get("user_id")).transpose()?)
}

/// Count of users in an environment, for the overview.
pub async fn count_users_in_environment(
    scope: &mut TenantScope<'_>,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_scopes WHERE environment_id = $1",
    )
    .bind(scope.environment_id())
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}
