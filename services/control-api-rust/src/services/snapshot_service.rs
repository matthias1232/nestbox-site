//! Snapshots on a timer, and clearing out old ones (ADR 0049).
//!
//! A sandbox that works on a joined node is snapshotted every few minutes, so losing the machine
//! loses minutes and not the work, and so the last move home has little left to send. Pruning runs
//! where the snapshot server's files are, because the server is append-only and no node may delete.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::move_service::{repository_password, repository_url};
use super::{require, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SnapshotView {
    pub id: Uuid,
    pub root_id: Uuid,
    pub snapshot: String,
    pub kind: String,
    pub bytes_added: i64,
    pub taken_at: DateTime<Utc>,
}

/// Whether a folder is due: nothing yet, or the last snapshot is older than the interval.
pub fn due(last: Option<DateTime<Utc>>, now: DateTime<Utc>, interval_minutes: i64) -> bool {
    match last {
        None => true,
        Some(at) => (now - at).num_minutes() >= interval_minutes,
    }
}

/// One pass: snapshot every folder that is mounted in a running sandbox on a joined node and is
/// due. A failing node is skipped and tried again next pass.
pub async fn tick(ctx: &ServiceContext, config: &Config, interval_minutes: i64) -> Result<usize, ApiError> {
    let Some(router) = ctx.router.clone() else { return Ok(0) };
    let rows: Vec<(Uuid, Uuid, Uuid, String, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT root_id, environment_id, node_id, host_path, last_snapshot FROM nb_snapshot_candidates()")
            .fetch_all(&ctx.pool)
            .await
            .map_err(db)?;
    let now = Utc::now();
    let mut taken = 0;
    for (root, environment_id, node_id, path, last) in rows {
        if !due(last, now, interval_minutes) {
            continue;
        }
        let adapter = router.for_node(Some(node_id));
        let Ok(password) = system_password(ctx, environment_id).await else { continue };
        let repo = repository_url(config, environment_id);
        match adapter.snapshot_backup(&repo, &password, &path, &["scheduled".to_string()]).await {
            Ok(fact) => {
                sqlx::query("INSERT INTO root_snapshots (environment_id, root_id, node_id, snapshot, kind, bytes_added) VALUES ($1, $2, $3, $4, 'scheduled', $5)")
                    .bind(environment_id)
                    .bind(root)
                    .bind(node_id)
                    .bind(&fact.snapshot)
                    .bind(fact.bytes_added)
                    .execute(&ctx.pool)
                    .await
                    .map_err(db)?;
                taken += 1;
            }
            Err(error) => tracing::debug!(root = %root, error = %error, "a scheduled snapshot was skipped"),
        }
    }
    Ok(taken)
}

async fn system_password(ctx: &ServiceContext, environment_id: Uuid) -> Result<String, ApiError> {
    super::secret_service::reveal(ctx, environment_id, "restic-repository").await
}

/// The snapshots of one folder, newest first.
pub async fn list(ctx: &ServiceContext, principal: &Principal, root_id: Uuid) -> Result<Vec<SnapshotView>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    // Visible only if the folder is: the lookup runs under the caller's row-level security.
    let visible: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM persistent_roots WHERE id = $1)")
        .bind(root_id)
        .fetch_one(scope.connection())
        .await
        .map_err(db)?;
    if !visible {
        return Err(ApiError::not_found(ErrorCode::SANDBOX_NOT_FOUND, "folder"));
    }
    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, String, i64, DateTime<Utc>)>(
        "SELECT id, root_id, snapshot, kind, bytes_added, taken_at FROM root_snapshots
          WHERE root_id = $1 AND environment_id = $2 ORDER BY taken_at DESC LIMIT 100",
    )
    .bind(root_id)
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(rows.into_iter().map(|(id, root_id, snapshot, kind, bytes_added, taken_at)| SnapshotView { id, root_id, snapshot, kind, bytes_added, taken_at }).collect())
}

/// Forget old snapshots of this data center's repository and reclaim the space. Administrators only.
pub async fn prune(ctx: &ServiceContext, principal: &Principal, keep_last: i32, keep_daily: i32) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let password = repository_password(ctx, principal).await?;
    // The local runner: it is the one that sees the snapshot server's files.
    let local = ctx.runtime_for(Uuid::nil());
    local
        .prune_snapshots(&principal.environment_id.to_string(), &password, keep_last.clamp(1, 1000), keep_daily.clamp(0, 366))
        .await
        .map_err(|e| ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_due_when_never_snapshotted_or_the_interval_has_passed() {
        let now = Utc::now();
        assert!(due(None, now, 15));
        assert!(!due(Some(now - chrono::Duration::minutes(5)), now, 15));
        assert!(due(Some(now - chrono::Duration::minutes(15)), now, 15));
    }
}
