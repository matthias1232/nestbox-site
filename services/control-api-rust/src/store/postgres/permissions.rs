//! Roles per scope: who is a member of a data center, a sandbox or an organisation.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::permissions::PermissionRow;
use crate::error::StoreError;

fn row_of(row: &sqlx::postgres::PgRow) -> Result<PermissionRow, StoreError> {
    Ok(PermissionRow {
        user_id: row.try_get("user_id")?,
        email: row.try_get("email")?,
        display_name: row.try_get("display_name")?,
        role: row.try_get("role")?,
        via: row.try_get("via")?,
        since: row.try_get("since")?,
    })
}

/// Everyone with a role in the scope's data center: direct `user_scopes` rows, plus people who
/// reach it only through the organisation tree (`nb_user_environments`).
pub async fn list_environment(scope: &mut TenantScope<'_>) -> Result<Vec<PermissionRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.email, u.display_name, x.role,
                CASE WHEN s.user_id IS NULL THEN 'organization' ELSE 'direct' END AS via,
                s.created_at AS since
           FROM users u
          CROSS JOIN LATERAL nb_user_environments(u.id) x
           LEFT JOIN user_scopes s ON s.user_id = u.id AND s.environment_id = $1
          WHERE x.environment_id = $1 AND u.status = 'active'
          ORDER BY u.email",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_of).collect()
}

/// The owner and every grantee of one sandbox.
pub async fn list_sandbox(scope: &mut TenantScope<'_>, sandbox: Uuid) -> Result<Vec<PermissionRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.email, u.display_name, 'member' AS role, 'owner' AS via,
                sb.created_at AS since
           FROM sandboxes sb JOIN users u ON u.id = sb.owner_user_id
          WHERE sb.id = $2 AND sb.environment_id = $1
         UNION ALL
         SELECT u.id, u.email, u.display_name, 'member', 'grant', g.created_at
           FROM sandbox_grants g JOIN users u ON u.id = g.user_id
          WHERE g.sandbox_id = $2 AND g.environment_id = $1
          ORDER BY 2",
    )
    .bind(scope.environment_id())
    .bind(sandbox)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(row_of).collect()
}

/// The administrators of an organisation and of every organisation above it: the people the
/// organisation tree gives reach into it. Users are not tenant data, so a pool is enough.
pub async fn list_organization(pool: &sqlx::PgPool, organization: Uuid) -> Result<Vec<PermissionRow>, StoreError> {
    let rows = sqlx::query(
        "WITH RECURSIVE chain AS (
             SELECT id, parent_id FROM organizations WHERE id = $1
             UNION
             SELECT o.id, o.parent_id FROM organizations o JOIN chain c ON o.id = c.parent_id
         )
         SELECT u.id AS user_id, u.email, u.display_name, u.role, 'organization' AS via,
                u.created_at AS since
           FROM users u JOIN chain c ON c.id = u.organization_id
          WHERE u.role = 'admin' AND u.status = 'active'
          ORDER BY u.email",
    )
    .bind(organization)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_of).collect()
}

/// Whether the caller reaches an organisation: it is their own, or the organisation of a data
/// center they reach.
pub async fn organization_reachable(
    pool: &sqlx::PgPool,
    user: Uuid,
    organization: Uuid,
) -> Result<bool, StoreError> {
    let found: Option<i32> = sqlx::query_scalar(
        "SELECT 1 WHERE EXISTS (SELECT 1 FROM users WHERE id = $1 AND organization_id = $2)
             OR EXISTS (SELECT 1 FROM nb_user_environments($1) r
                          JOIN environments e ON e.id = r.environment_id
                         WHERE e.organization_id = $2)",
    )
    .bind(user)
    .bind(organization)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

/// Whether a user may be given a role in the scope's data center: they are active and belong
/// to the data center's organisation or one above it.
pub async fn user_may_join(scope: &mut TenantScope<'_>, user: Uuid) -> Result<bool, StoreError> {
    let found: Option<i32> = sqlx::query_scalar(
        "WITH RECURSIVE chain AS (
             SELECT o.id, o.parent_id FROM environments e
               JOIN organizations o ON o.id = e.organization_id WHERE e.id = $1
             UNION
             SELECT p.id, p.parent_id FROM organizations p JOIN chain c ON p.id = c.parent_id
         )
         SELECT 1 FROM users u JOIN chain c ON c.id = u.organization_id
          WHERE u.id = $2 AND u.status = 'active'",
    )
    .bind(scope.environment_id())
    .bind(user)
    .fetch_optional(scope.connection())
    .await?;
    Ok(found.is_some())
}

/// The organisation a data center belongs to.
pub async fn organization_of_environment(
    scope: &mut TenantScope<'_>,
) -> Result<Option<Uuid>, StoreError> {
    let id: Option<Uuid> = sqlx::query_scalar("SELECT organization_id FROM environments WHERE id = $1")
        .bind(scope.environment_id())
        .fetch_optional(scope.connection())
        .await?;
    Ok(id)
}

/// The role a user has in a data center through a direct `user_scopes` row.
pub async fn direct_role(scope: &mut TenantScope<'_>, user: Uuid) -> Result<Option<String>, StoreError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM user_scopes WHERE user_id = $1 AND environment_id = $2")
            .bind(user)
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await?;
    Ok(role)
}

/// How many other people hold a direct admin role in the scope's data center.
pub async fn other_direct_admins(scope: &mut TenantScope<'_>, user: Uuid) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_scopes s JOIN users u ON u.id = s.user_id
          WHERE s.environment_id = $1 AND s.role = 'admin' AND s.user_id <> $2 AND u.status = 'active'",
    )
    .bind(scope.environment_id())
    .bind(user)
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}

/// Write one role in the scope's data center (insert or change).
pub async fn set_scope(scope: &mut TenantScope<'_>, user: Uuid, role: &str) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO user_scopes (user_id, environment_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, environment_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(user)
    .bind(scope.environment_id())
    .bind(role)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Remove one role from the scope's data center; false when there was none.
pub async fn remove_scope(scope: &mut TenantScope<'_>, user: Uuid) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM user_scopes WHERE user_id = $1 AND environment_id = $2")
        .bind(user)
        .bind(scope.environment_id())
        .execute(scope.connection())
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Open invitations of the scope's data center; with a sandbox, only those for that sandbox,
/// otherwise only the ones that are not tied to a sandbox.
pub async fn list_invitations(
    scope: &mut TenantScope<'_>,
    sandbox: Option<Uuid>,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, role, email, expires_at, sandbox_id FROM invitations
          WHERE environment_id = $1 AND accepted_at IS NULL AND expires_at > now()
            AND (($2::uuid IS NULL AND sandbox_id IS NULL) OR sandbox_id = $2)
          ORDER BY created_at DESC",
    )
    .bind(scope.environment_id())
    .bind(sandbox)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        let id: Uuid = row.try_get("id")?;
        let role: String = row.try_get("role")?;
        let email: Option<String> = row.try_get("email")?;
        let expires_at: chrono::DateTime<chrono::Utc> = row.try_get("expires_at")?;
        let sandbox_id: Option<Uuid> = row.try_get("sandbox_id")?;
        out.push(serde_json::json!({
            "id": id, "role": role, "email": email, "expires_at": expires_at, "sandbox_id": sandbox_id,
        }));
    }
    Ok(out)
}
