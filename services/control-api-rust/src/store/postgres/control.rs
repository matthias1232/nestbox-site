//! The workbox's read models: every sandbox's controls at a glance (ADR 0053).

use sqlx::Row;

use super::super::TenantScope;
use crate::error::StoreError;

/// One sandbox's name and its node's name and tunnel state.
pub async fn node_brief(
    scope: &mut TenantScope<'_>,
    node_id: uuid::Uuid,
) -> Result<Option<(String, Option<String>, Option<String>)>, StoreError> {
    let row = sqlx::query(
        "SELECT name, tunnel_state, public_endpoint FROM nodes
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(node_id)
    .fetch_optional(scope.connection())
    .await?;
    Ok(row.map(|row| {
        (
            row.get::<String, _>("name"),
            row.get::<Option<String>, _>("tunnel_state"),
            row.get::<Option<String>, _>("public_endpoint"),
        )
    }))
}

/// Every sandbox of this environment with its controls, as one JSON array.
///
/// One query rather than one per sandbox: the list is shown for every sandbox at
/// once, and a hundred round trips would be a page that loads slowly for no
/// reason. Row-level security still decides which sandboxes come back.
pub async fn overview(scope: &mut TenantScope<'_>) -> Result<serde_json::Value, StoreError> {
    let row = sqlx::query(
        "SELECT coalesce(json_agg(item), '[]'::json) AS rows FROM (
            SELECT json_build_object(
                'sandbox_id', s.id,
                'name', s.name,
                'owner_name', u.display_name,
                'node_name', n.name,
                'status', s.status,
                'egress_mode', coalesce(em.mode, 'forward_proxy'),
                'allow_all', coalesce(em.mode = 'off_allow_all', false) OR coalesce(w.window_count, 0) > 0,
                'allow_all_until', CASE WHEN w.unlimited THEN NULL ELSE w.latest_expiry END,
                'agents', coalesce(aa.agents, '[]'::json),
                'viewers', coalesce(vv.viewers, '[]'::json)
              ) AS item
              FROM sandboxes s
              LEFT JOIN users u ON u.id = s.owner_user_id
              LEFT JOIN nodes n ON n.id = s.node_id
              LEFT JOIN egress_modes em ON em.sandbox_id = s.id AND em.environment_id = s.environment_id
              LEFT JOIN LATERAL (
                  SELECT count(*) AS window_count,
                         bool_or(w2.expires_at IS NULL) AS unlimited,
                         max(w2.expires_at) AS latest_expiry
                    FROM traffic_windows w2
                   WHERE w2.environment_id = s.environment_id
                     AND w2.revoked_at IS NULL
                     AND (w2.expires_at IS NULL OR w2.expires_at > now())
                     AND (w2.sandbox_id IS NULL OR w2.sandbox_id = s.id)
              ) w ON true
              LEFT JOIN LATERAL (
                  SELECT json_agg(json_build_object(
                             'agent', x.agent, 'mode', x.mode,
                             'pending_restart', x.pending_restart)) AS agents
                    FROM sandbox_agent_auth x
                   WHERE x.environment_id = s.environment_id AND x.sandbox_id = s.id
              ) aa ON true
              LEFT JOIN LATERAL (
                  SELECT json_agg(json_build_object(
                             'user_name', coalesce(uu.display_name, 'SSH'),
                             'kind', ss.kind)) AS viewers
                    FROM sandbox_sessions ss
                    LEFT JOIN users uu ON uu.id = ss.user_id
                   WHERE ss.environment_id = s.environment_id AND ss.sandbox_id = s.id
                     AND ss.closed_at IS NULL
                     AND ss.opened_at > now() - interval '35 minutes'
              ) vv ON true
             WHERE s.environment_id = $1
             ORDER BY s.name
        ) rows",
    )
    .bind(scope.environment_id())
    .fetch_one(scope.connection())
    .await?;
    Ok(row.try_get("rows")?)
}
