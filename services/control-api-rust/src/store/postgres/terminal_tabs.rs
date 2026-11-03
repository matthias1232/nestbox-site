//! The named terminal tabs one user keeps in one sandbox.
//!
//! A tab is a promise: the id names a tmux session inside the sandbox, and the
//! console reopens it after a reload instead of starting a new shell. The row is
//! the server's copy of that promise, so the browser is not the only place it
//! exists. No keystroke and no output is ever stored here (ADR 0023).

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::error::StoreError;

/// The most tabs one user may keep in one sandbox.
///
/// The database bounds the column; this bounds the set. Four terminals may be
/// open at once (ADR 0023), but a user may keep more named tabs than that — a
/// tab whose shell ended is still a place to return to.
pub const MAX_TABS: usize = 16;

/// One stored tab.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TabRow {
    pub tab_id: String,
    pub title: String,
    pub position: i32,
    pub last_seen_at: DateTime<Utc>,
}

/// The tabs one user keeps in one sandbox, in the order the browser showed them.
pub async fn list(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<TabRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT tab_id, title, position, last_seen_at
           FROM sandbox_terminal_tabs
          WHERE environment_id = $1 AND sandbox_id = $2 AND user_id = $3
          ORDER BY position ASC, created_at ASC",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(user_id)
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TabRow {
            tab_id: row.try_get("tab_id")?,
            title: row.try_get("title")?,
            position: row.try_get("position")?,
            last_seen_at: row.try_get("last_seen_at")?,
        });
    }
    Ok(out)
}

/// Replace the tab set of one user in one sandbox.
///
/// Replace rather than merge: the console sends the list it is showing, and a
/// tab the operator closed must disappear. `tabs` is already validated and
/// bounded by the service; this function only writes it.
pub async fn replace(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    user_id: Uuid,
    tabs: &[(String, String)],
) -> Result<Vec<TabRow>, StoreError> {
    sqlx::query(
        "DELETE FROM sandbox_terminal_tabs
          WHERE environment_id = $1 AND sandbox_id = $2 AND user_id = $3",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(user_id)
    .execute(scope.connection())
    .await?;

    for (position, (tab_id, title)) in tabs.iter().enumerate() {
        sqlx::query(
            "INSERT INTO sandbox_terminal_tabs
                (environment_id, sandbox_id, user_id, tab_id, title, position)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(scope.environment_id())
        .bind(sandbox_id)
        .bind(user_id)
        .bind(tab_id)
        .bind(title)
        .bind(position as i32)
        .execute(scope.connection())
        .await?;
    }

    list(scope, sandbox_id, user_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tab_cap_matches_what_a_browser_can_hold() {
        // The console adds one tab at a time and never trims; the cap is the
        // server's, so a runaway client cannot grow the table.
        assert_eq!(MAX_TABS, 16);
    }
}

/// One tab of a sandbox, with who created it: the shared view of the tab strip.
#[derive(Debug, Clone)]
pub struct SharedTab {
    /// The tab id, the tmux session name after `nestbox-`.
    pub tab_id: String,
    /// The title the creator gave it.
    pub title: String,
    /// The creator's display name, when known.
    pub created_by_name: Option<String>,
}

/// Every user's tabs of one sandbox, one row per tab id.
pub async fn list_all(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
) -> Result<Vec<SharedTab>, StoreError> {
    let rows = sqlx::query(
        "SELECT DISTINCT ON (t.tab_id) t.tab_id, t.title, u.display_name AS created_by_name
           FROM sandbox_terminal_tabs t
           LEFT JOIN users u ON u.id = t.user_id
          WHERE t.environment_id = $1 AND t.sandbox_id = $2
          ORDER BY t.tab_id, t.created_at",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_all(scope.connection())
    .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(SharedTab {
            tab_id: row.try_get("tab_id")?,
            title: row.try_get("title")?,
            created_by_name: row.try_get("created_by_name")?,
        });
    }
    Ok(out)
}

/// Remove one tab for every user of the sandbox.
pub async fn delete_for_all(
    scope: &mut TenantScope<'_>,
    sandbox_id: Uuid,
    tab_id: &str,
) -> Result<u64, StoreError> {
    let affected = sqlx::query(
        "DELETE FROM sandbox_terminal_tabs
          WHERE environment_id = $1 AND sandbox_id = $2 AND tab_id = $3",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .bind(tab_id)
    .execute(scope.connection())
    .await?
    .rows_affected();
    Ok(affected)
}
