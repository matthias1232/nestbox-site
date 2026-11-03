//! Refused destinations grouped into alerts (package B08).

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::alerts::{alert_id, AlertView};
use crate::error::StoreError;

/// The most alerts one call returns; an operator cannot read more than this anyway.
const MAX_ALERTS: i64 = 200;

/// Refused destinations since `since`, grouped per sandbox, host, port and channel.
///
/// Same grouping as the decisions view (`traffic::denied_targets`) so an alert and a decision
/// row describe the same destination. A group is dropped when somebody answered it: an open
/// traffic window of the sandbox created after its last refusal (channel `all` or its own), or
/// an enabled allow rule for the host created after its first refusal (a quick allow is such a
/// rule). The owner's display name comes from `sandboxes.owner_user_id`.
pub async fn refused_grouped(
    scope: &mut TenantScope<'_>,
    since: DateTime<Utc>,
    sandbox: Option<Uuid>,
) -> Result<Vec<AlertView>, StoreError> {
    let sql = "WITH base AS (
            SELECT e.sandbox_id,
                   s.name AS sandbox_name,
                   u.display_name AS owner,
                   e.ts,
                   COALESCE(NULLIF(e.detail->>'host', ''), NULLIF(e.detail->>'destination', ''),
                            NULLIF(e.detail->>'hostname', '')) AS host,
                   CASE WHEN COALESCE(e.detail->>'port', '') ~ '^[0-9]{1,5}$'
                        THEN (e.detail->>'port')::integer ELSE 0 END AS port,
                   COALESCE(NULLIF(e.detail->>'protocol', ''), 'tcp') AS protocol,
                   COALESCE(NULLIF(e.detail->>'channel', ''), 'proxy') AS channel,
                   COALESCE(NULLIF(e.detail->>'reason', ''), 'no_rule') AS reason,
                   COALESCE((e.detail->>'would_deny')::boolean, false) AS would_deny
              FROM telemetry_events e
              JOIN sandboxes s ON s.id = e.sandbox_id AND s.environment_id = e.environment_id
              LEFT JOIN users u ON u.id = s.owner_user_id
             WHERE e.environment_id = $1
               AND e.ts >= $2
               AND e.sandbox_id IS NOT NULL
               AND e.kind IN ('network', 'dns')
               AND COALESCE(e.detail->>'decision', '') = 'deny'
               AND ($3::uuid IS NULL OR e.sandbox_id = $3::uuid)
         ),
         grouped AS (
            SELECT sandbox_id, sandbox_name, max(owner) AS owner, lower(host) AS host, port,
                   (array_agg(protocol ORDER BY ts DESC))[1] AS protocol,
                   channel,
                   COUNT(*) AS count,
                   MIN(ts) AS first_at,
                   MAX(ts) AS last_at,
                   (array_agg(reason ORDER BY ts DESC))[1] AS reason,
                   bool_or(would_deny) AS would_deny
              FROM base
             WHERE host IS NOT NULL
             GROUP BY sandbox_id, sandbox_name, lower(host), port, channel
         )
         SELECT g.*
           FROM grouped g
          WHERE NOT EXISTS (
                SELECT 1 FROM traffic_windows w
                 WHERE w.environment_id = $1
                   AND (w.sandbox_id = g.sandbox_id OR w.scope = 'environment')
                   AND w.revoked_at IS NULL
                   AND (w.expires_at IS NULL OR w.expires_at > now())
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
          ORDER BY g.last_at DESC
          LIMIT $4";

    let rows = sqlx::query(sql)
        .bind(scope.environment_id())
        .bind(since)
        .bind(sandbox)
        .bind(MAX_ALERTS)
        .fetch_all(scope.connection())
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let sandbox_id: Uuid = row.try_get("sandbox_id")?;
        let host: String = row.try_get("host")?;
        let port: i32 = row.try_get("port")?;
        out.push(AlertView {
            id: alert_id(sandbox_id, &host, port),
            sandbox_id,
            sandbox_name: row.try_get("sandbox_name")?,
            host,
            port,
            protocol: row.try_get("protocol")?,
            channel: row.try_get("channel")?,
            count: row.try_get("count")?,
            first_at: row.try_get("first_at")?,
            last_at: row.try_get("last_at")?,
            decision: "blocked".to_string(),
            reason: row.try_get("reason")?,
            would_deny: row.try_get("would_deny")?,
            owner: row.try_get("owner")?,
        });
    }
    Ok(out)
}
