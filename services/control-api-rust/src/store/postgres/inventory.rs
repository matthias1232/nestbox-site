//! Flat rows for the inventory tree (package B01).

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::inventory::{ClusterRow, EnvironmentRow, InstallRow, PlacementRow, UserRow};
use crate::error::StoreError;

/// The data centers with their organisation, for the ids a caller reaches.
///
/// `organizations` and `environments` are not tenant data, so this runs on the pool, the same
/// way the organisation list does.
pub async fn environment_rows(
    pool: &sqlx::PgPool,
    environment_ids: &[Uuid],
) -> Result<Vec<EnvironmentRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT e.id AS environment_id, e.name AS environment_name,
                o.id AS organization_id, o.name AS organization_name,
                o.parent_id AS organization_parent, o.kind AS organization_kind
           FROM environments e
           JOIN organizations o ON o.id = e.organization_id
          WHERE e.id = ANY($1)
          ORDER BY e.name, e.id",
    )
    .bind(environment_ids)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(EnvironmentRow {
            environment_id: row.try_get("environment_id")?,
            environment_name: row.try_get("environment_name")?,
            organization_id: row.try_get("organization_id")?,
            organization_name: row.try_get("organization_name")?,
            organization_parent: row.try_get("organization_parent")?,
            organization_kind: row.try_get("organization_kind")?,
        });
    }
    Ok(out)
}

/// Every site of the scope's data center with its nodes and sandboxes, one row per leaf.
///
/// One `LEFT JOIN` chain, so a site without nodes and a node without sandboxes are rows of
/// their own. Refused destinations of the last 15 minutes are grouped exactly as the alert list
/// groups them (`alerts::refused_grouped`) and only the unanswered groups are counted, so the
/// badge in the tree agrees with the list and the dock. Budgets are not read here (`ram_free` and
/// `vram_free` are `None`): the service composes them from the node budget.
pub async fn placement_rows(scope: &mut TenantScope<'_>) -> Result<Vec<PlacementRow>, StoreError> {
    let rows = sqlx::query(
        "WITH refused AS (
             SELECT e.sandbox_id,
                    lower(COALESCE(NULLIF(e.detail->>'host', ''),
                                   NULLIF(e.detail->>'destination', ''),
                                   NULLIF(e.detail->>'hostname', ''))) AS host,
                    CASE WHEN COALESCE(e.detail->>'port', '') ~ '^[0-9]{1,5}$'
                         THEN (e.detail->>'port')::integer ELSE 0 END AS port,
                    COALESCE(NULLIF(e.detail->>'channel', ''), 'proxy') AS channel,
                    e.ts
               FROM telemetry_events e
              WHERE e.environment_id = $1
                AND e.ts >= now() - interval '15 minutes'
                AND e.kind IN ('network', 'dns')
                AND e.sandbox_id IS NOT NULL
                AND COALESCE(e.detail->>'decision', '') = 'deny'
         ),
         grouped AS (
             SELECT sandbox_id, host, port, channel, min(ts) AS first_at, max(ts) AS last_at
               FROM refused
              WHERE host IS NOT NULL
              GROUP BY sandbox_id, host, port, channel
         ),
         blocked AS (
             SELECT g.sandbox_id, count(*)::bigint AS n
               FROM grouped g
              WHERE NOT EXISTS (
                    SELECT 1 FROM traffic_windows w
                     WHERE w.environment_id = $1
                       AND (w.sandbox_id = g.sandbox_id OR w.scope = 'environment')
                       AND w.revoked_at IS NULL
                       AND w.expires_at > now()
                       AND w.channel IN ('all', g.channel)
                       AND w.created_at > g.last_at)
                AND NOT EXISTS (
                    SELECT 1 FROM network_rules r
                     WHERE r.environment_id = $1
                       AND r.sandbox_id = g.sandbox_id
                       AND r.action = 'allow'
                       AND r.enabled
                       AND lower(r.host) = g.host
                       AND (r.port IS NULL OR r.port = g.port)
                       AND r.created_at > g.first_at)
              GROUP BY g.sandbox_id
         )
         SELECT si.id AS site_id, si.name AS site_name, si.kind AS site_kind,
                n.id AS node_id, n.name AS node_name, n.status AS node_status,
                n.tunnel_state, COALESCE(n.container_gpu, false) AS gpu,
                sb.id AS sandbox_id, sb.name AS sandbox_name, sb.status AS sandbox_status,
                sb.pack_key, sb.owner_user_id AS owner_id, u.display_name AS owner_name,
                em.mode AS egress_mode,
                (COALESCE(em.mode = 'off_allow_all', false) OR EXISTS (
                    SELECT 1 FROM traffic_windows tw
                     WHERE tw.environment_id = $1
                       AND (tw.sandbox_id = sb.id OR tw.scope = 'environment')
                       AND tw.revoked_at IS NULL
                       AND (tw.expires_at IS NULL OR tw.expires_at > now())
                       AND tw.channel IN ('all', 'proxy')
                       AND tw.mode = 'bypass'
                )) AS allow_all,
                COALESCE(b.n, 0)::bigint AS alerts
           FROM (SELECT id, environment_id, name, kind FROM sites
                 UNION ALL
                 -- A node without a site (the local node) sits under a placeholder site with
                 -- the nil id, so it is not dropped from the tree.
                 SELECT '00000000-0000-0000-0000-000000000000'::uuid, $1::uuid, 'No site', NULL) si
           LEFT JOIN nodes n
                  ON n.environment_id = si.environment_id
                 AND (n.site_id = si.id
                      OR (n.site_id IS NULL AND si.id = '00000000-0000-0000-0000-000000000000'::uuid))
           LEFT JOIN sandboxes sb
                  ON sb.node_id = n.id AND sb.environment_id = si.environment_id
                 AND sb.status <> 'deleted'
           LEFT JOIN egress_modes em
                  ON em.sandbox_id = sb.id AND em.environment_id = si.environment_id
           LEFT JOIN users u ON u.id = sb.owner_user_id
           LEFT JOIN blocked b ON b.sandbox_id = sb.id
          WHERE si.environment_id = $1
            AND (si.id <> '00000000-0000-0000-0000-000000000000'::uuid OR n.id IS NOT NULL)
          ORDER BY si.name, n.name, sb.name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let environment_id = scope.environment_id();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(PlacementRow {
            environment_id,
            site_id: row.try_get("site_id")?,
            site_name: row.try_get("site_name")?,
            site_kind: row.try_get("site_kind")?,
            node_id: row.try_get("node_id")?,
            node_name: row.try_get("node_name")?,
            node_status: row.try_get("node_status")?,
            tunnel_state: row.try_get("tunnel_state")?,
            gpu: row.try_get("gpu")?,
            ram_free: None,
            vram_free: None,
            sandbox_id: row.try_get("sandbox_id")?,
            sandbox_name: row.try_get("sandbox_name")?,
            sandbox_status: row.try_get("sandbox_status")?,
            pack_key: row.try_get("pack_key")?,
            owner_id: row.try_get("owner_id")?,
            owner_name: row.try_get("owner_name")?,
            egress_mode: row.try_get("egress_mode")?,
            allow_all: row.try_get("allow_all")?,
            alerts: row.try_get("alerts")?,
        });
    }
    Ok(out)
}

/// The people of the scope's data center with their role there, for the workloads view.
pub async fn user_rows(scope: &mut TenantScope<'_>) -> Result<Vec<UserRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.display_name, us.role
           FROM user_scopes us
           JOIN users u ON u.id = us.user_id
          WHERE us.environment_id = $1 AND u.status = 'active'
          ORDER BY u.display_name, u.id",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let environment_id = scope.environment_id();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(UserRow {
            environment_id,
            user_id: row.try_get("user_id")?,
            display_name: row.try_get("display_name")?,
            role: row.try_get("role")?,
        });
    }
    Ok(out)
}

/// Model installs of the scope's data center for the models view, optionally for one model.
///
/// Each carries the node name, the provider profile's status and the cluster it belongs to.
pub async fn install_rows(
    scope: &mut TenantScope<'_>,
    model_key: Option<&str>,
) -> Result<Vec<InstallRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT i.id AS install_id, i.model_key, m.name AS model_name, m.family AS model_family,
                i.node_id, n.name AS node_name, i.state,
                p.status AS provider_status, cm.cluster_id
           FROM model_installs i
           JOIN models m ON m.key = i.model_key
           JOIN nodes n ON n.id = i.node_id
           LEFT JOIN provider_profiles p ON p.id = i.provider_profile_id
           LEFT JOIN LATERAL (
                SELECT cluster_id FROM model_cluster_members
                 WHERE install_id = i.id ORDER BY added_at, cluster_id LIMIT 1
           ) cm ON true
          WHERE i.environment_id = $1
            AND ($2::text IS NULL OR i.model_key = $2)
          ORDER BY m.name, n.name, i.id",
    )
    .bind(scope.environment_id())
    .bind(model_key)
    .fetch_all(scope.connection())
    .await?;
    let environment_id = scope.environment_id();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(InstallRow {
            install_id: row.try_get("install_id")?,
            environment_id,
            model_key: row.try_get("model_key")?,
            model_name: row.try_get("model_name")?,
            model_family: row.try_get("model_family")?,
            node_id: row.try_get("node_id")?,
            node_name: row.try_get("node_name")?,
            state: row.try_get("state")?,
            provider_status: row.try_get("provider_status")?,
            cluster_id: row.try_get("cluster_id")?,
        });
    }
    Ok(out)
}

/// Model clusters of the scope's data center, for the models view.
pub async fn cluster_rows(scope: &mut TenantScope<'_>) -> Result<Vec<ClusterRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT id AS cluster_id, name, model_key, strategy
           FROM model_clusters
          WHERE environment_id = $1
          ORDER BY name, id",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;
    let environment_id = scope.environment_id();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ClusterRow {
            cluster_id: row.try_get("cluster_id")?,
            environment_id,
            name: row.try_get("name")?,
            model_key: row.try_get("model_key")?,
            strategy: row.try_get("strategy")?,
        });
    }
    Ok(out)
}
