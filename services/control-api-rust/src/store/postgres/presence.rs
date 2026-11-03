//! Open sessions (`sandbox_sessions`) as presence: who is in which terminal tab
//! or desktop.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::presence::PresenceEntry;
use crate::error::StoreError;

/// Open a session row and return its id.
pub async fn open_session(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    user_id: Option<Uuid>,
    kind: &str,
    tab_id: Option<&str>,
    view_only: bool,
) -> Result<Uuid, StoreError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO sandbox_sessions
            (environment_id, sandbox_id, user_id, kind, tab_id, view_only)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(user_id)
    .bind(kind)
    .bind(tab_id)
    .bind(view_only)
    .fetch_one(scope.connection())
    .await?;
    Ok(id)
}

/// Close a session row.
pub async fn close_session(scope: &mut TenantScope<'_>, id: Uuid, reason: &str) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE sandbox_sessions SET closed_at = now(), close_reason = $3
          WHERE environment_id = $1 AND id = $2 AND closed_at IS NULL",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(reason)
    .execute(scope.connection())
    .await?;
    Ok(())
}

/// Sessions of a sandbox that are open and younger than `stale_seconds`.
pub async fn list_open(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    stale_seconds: i64,
) -> Result<Vec<PresenceEntry>, StoreError> {
    let rows = sqlx::query(
        "SELECT s.id, s.user_id, u.display_name AS user_name, s.kind, s.tab_id,
                s.view_only, s.opened_at
           FROM sandbox_sessions s
           LEFT JOIN users u ON u.id = s.user_id
          WHERE s.environment_id = $1 AND s.sandbox_id = $2 AND s.closed_at IS NULL
            AND s.opened_at > now() - make_interval(secs => $3)
          ORDER BY s.opened_at",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(stale_seconds as f64)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(PresenceEntry {
            session_id: row.try_get("id")?,
            user_id: row.try_get("user_id")?,
            user_name: row.try_get("user_name")?,
            kind: row.try_get("kind")?,
            tab_id: row.try_get("tab_id")?,
            view_only: row.try_get("view_only")?,
            opened_at: row.try_get("opened_at")?,
        });
    }
    Ok(out)
}
