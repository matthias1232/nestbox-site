//! Sandbox queries.
//!
//! The list query is the one that has to hold up at ten thousand rows, so it is
//! written as keyset paging with a stable sort and a `LIMIT`, never `OFFSET`.
//! The filter predicates take optional parameters, which keeps one prepared
//! statement per sort order instead of a query built from strings.

use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::pagination::{Cursor, CursorKey, ListParams, Page, SortDirection};
use crate::domain::sandbox::{Sandbox, SandboxHealth, SandboxStatus};
use crate::error::StoreError;

/// A row as the database returns it, mapped into the domain type.
fn row_to_sandbox(row: &sqlx::postgres::PgRow) -> Result<Sandbox, StoreError> {
    let status_raw: String = row.try_get("status")?;
    let health_raw: String = row.try_get("health")?;
    let status = SandboxStatus::parse(&status_raw)
        .ok_or_else(|| StoreError::Database(sqlx::Error::Decode(status_raw.into())))?;
    let health = SandboxHealth::parse(&health_raw)
        .ok_or_else(|| StoreError::Database(sqlx::Error::Decode(health_raw.into())))?;

    Ok(Sandbox {
        id: row.try_get("id")?,
        environment_id: row.try_get("environment_id")?,
        cluster_id: row.try_get("cluster_id")?,
        gateway_id: row.try_get("gateway_id")?,
        gateway_name: row.try_get("gateway_name")?,
        supervisor_id: row.try_get("supervisor_id")?,
        workspace_id: row.try_get("workspace_id")?,
        parent_id: row.try_get("parent_id")?,
        node_id: row.try_get("node_id")?,
        node_name: row.try_get("node_name")?,
        name: row.try_get("name")?,
        pack_key: row.try_get("pack_key")?,
        image: row.try_get("image")?,
        image_digest: row.try_get("image_digest")?,
        spec: row.try_get("spec")?,
        status,
        health,
        version: row.try_get("version")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        spec_detail: None,
    })
}

/// The column set every sandbox read uses. Named, never `*`, so a schema change
/// breaks loudly in the mapping rather than silently mis-aligning a field.
const SANDBOX_COLUMNS: &str = "
    s.id, s.environment_id, s.cluster_id, s.gateway_id, s.supervisor_id,
    s.workspace_id, s.parent_id, s.node_id, s.name, s.pack_key, s.image, s.image_digest, s.spec,
    s.status, s.health, s.version, s.created_at, s.updated_at,
    g.name AS gateway_name, n.name AS node_name
";

/// List sandboxes with keyset paging and facets.
///
/// The `total_estimate` is the sum of the status facet, which is one indexed
/// `GROUP BY` per page rather than a separate `COUNT(*)`. The facet query is
/// already paid for (ADR 0006), so the total comes free and is exact.
pub async fn list(
    scope: &mut TenantScope<'_>,
    params: &ListParams,
) -> Result<Page<Sandbox>, StoreError> {
    let environment_id = scope.environment_id();

    // Only enum-derived identifiers reach the SQL text. A user-supplied sort is
    // parsed into this enum before it gets here.
    // Qualified with the table alias: the join brings a second `updated_at`
    // into scope, and an unqualified column there is ambiguous.
    let order_column = format!("s.{}", params.sort.column());
    let direction = params.direction.sql();
    let comparison = match params.direction {
        SortDirection::Desc => "<",
        SortDirection::Asc => ">",
    };

    let status_filter: Option<Vec<String>> = params
        .filters
        .get("status")
        .map(|raw| raw.split(',').map(str::trim).map(str::to_string).collect());
    let pack_filter = params.filters.get("pack").cloned();
    let gateway_filter = params.filters.get("gateway").cloned();

    // The node filter arrives as a resolved id: the API layer looks the name up
    // first, because an unknown node is a 404 and not an empty page. A value
    // that is not a uuid can therefore only be a caller bug, and it matches
    // nothing rather than widening the list.
    let node_filter = match params.filters.get("node_id").map(String::as_str) {
        Some(raw) => Some(Uuid::parse_str(raw).map_err(|_| StoreError::NotFound)?),
        None => None,
    };

    let (cursor_ts, cursor_text, cursor_id) = match &params.cursor {
        Some(Cursor {
            key: CursorKey::Timestamp(ts),
            id,
        }) => (Some(*ts), None::<String>, Some(*id)),
        Some(Cursor {
            key: CursorKey::Text(text),
            id,
        }) => (None, Some(text.clone()), Some(*id)),
        None => (None, None, None),
    };

    // Fetch one extra row to learn whether a next page exists without a second
    // count query.
    let fetch_limit = params.limit + 1;

    // Two shapes, because the cursor key has two types. A timestamp column
    // compared against a text parameter is a type error the planner raises even
    // when the parameter is null, so the branch has to exist in the SQL, not in
    // the data.
    let text_sorted = matches!(
        params.sort,
        crate::domain::pagination::SortField::Name
            | crate::domain::pagination::SortField::Status
    );

    // The two branches have the same parameter count and the same positions;
    // only the type of the cursor key differs. Keeping the numbering identical
    // means the bind list stays one shape, and no parameter is sent that the
    // statement does not reference — which Postgres rejects.
    let cursor_clause = if text_sorted {
        format!("($6::text IS NULL OR ({order_column}, s.id) {comparison} ($6, $7))")
    } else {
        format!("($6::timestamptz IS NULL OR ({order_column}, s.id) {comparison} ($6, $7))")
    };

    let sql = format!(
        "SELECT {SANDBOX_COLUMNS}
           FROM sandboxes s
           LEFT JOIN gateways g ON g.id = s.gateway_id
           LEFT JOIN nodes n ON n.id = s.node_id
          WHERE s.environment_id = $1
            AND ($2::text[] IS NULL OR s.status = ANY($2))
            AND ($3::text IS NULL OR s.pack_key = $3)
            AND ($4::text IS NULL OR g.name = $4)
            AND ($5::text IS NULL OR s.name ILIKE '%' || $5 || '%')
            AND ($9::uuid IS NULL OR s.node_id = $9)
            AND {cursor_clause}
          ORDER BY {order_column} {direction}, s.id {direction}
          LIMIT $8"
    );

    let mut query = sqlx::query(&sql)
        .bind(environment_id)
        .bind(status_filter.as_deref())
        .bind(pack_filter.as_deref())
        .bind(gateway_filter.as_deref())
        .bind(params.search.as_deref());

    // Exactly one of the two cursor keys is bound, at the position the branch
    // expects.
    query = if text_sorted {
        query.bind(cursor_text.as_deref())
    } else {
        query.bind(cursor_ts)
    };

    let rows = query
        .bind(cursor_id)
        .bind(fetch_limit)
        .bind(node_filter)
        .fetch_all(scope.connection())
        .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        items.push(row_to_sandbox(row)?);
    }

    let has_next = items.len() as i64 > params.limit;
    if has_next {
        items.truncate(params.limit as usize);
    }

    let next_cursor = if has_next {
        items.last().map(|last| {
            let key = match params.sort {
                crate::domain::pagination::SortField::Name => CursorKey::Text(last.name.clone()),
                crate::domain::pagination::SortField::Status => {
                    CursorKey::Text(last.status.as_str().to_string())
                }
                crate::domain::pagination::SortField::CreatedAt => {
                    CursorKey::Timestamp(last.created_at)
                }
                crate::domain::pagination::SortField::UpdatedAt => {
                    CursorKey::Timestamp(last.updated_at)
                }
            };
            Cursor { key, id: last.id }.encode()
        })
    } else {
        None
    };

    let facets = facets(scope).await?;
    // The status facet is already an exact per-tenant count (ADR 0006 asks for
    // facets on every page), so the total is its sum. The planner estimate was
    // table-wide, not per tenant, and reads zero until the first ANALYZE —
    // which made a page of two rows report "0 total".
    let total_estimate = facets.total_of("status");
    let _ = environment_id;

    Ok(Page {
        items,
        next_cursor,
        total_estimate,
        facets,
    })
}

/// Status and pack counts for the filter sidebar, in one round trip each.
async fn facets(scope: &mut TenantScope<'_>) -> Result<crate::domain::pagination::Facets, StoreError> {
    use crate::domain::pagination::Facets;
    let mut facets = Facets::default();
    let environment_id = scope.environment_id();

    let status_rows = sqlx::query(
        "SELECT status, COUNT(*) AS n FROM sandboxes WHERE environment_id = $1 GROUP BY status",
    )
    .bind(environment_id)
    .fetch_all(scope.connection())
    .await?;
    for row in status_rows {
        let status: String = row.try_get("status")?;
        let count: i64 = row.try_get("n")?;
        facets.add("status", &status, count);
    }

    let pack_rows = sqlx::query(
        "SELECT pack_key, COUNT(*) AS n FROM sandboxes WHERE environment_id = $1 GROUP BY pack_key",
    )
    .bind(environment_id)
    .fetch_all(scope.connection())
    .await?;
    for row in pack_rows {
        let pack: String = row.try_get("pack_key")?;
        let count: i64 = row.try_get("n")?;
        facets.add("pack", &pack, count);
    }

    Ok(facets)
}

/// Read one sandbox, including its desired-state document.
pub async fn get(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<Sandbox, StoreError> {
    let sql = format!(
        "SELECT {SANDBOX_COLUMNS}
           FROM sandboxes s
           LEFT JOIN gateways g ON g.id = s.gateway_id
           LEFT JOIN nodes n ON n.id = s.node_id
          WHERE s.environment_id = $1 AND s.id = $2"
    );
    let row = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(id)
        .fetch_optional(scope.connection())
        .await?
        .ok_or(StoreError::NotFound)?;

    let mut sandbox = row_to_sandbox(&row)?;
    sandbox.spec_detail = Some(sandbox.spec.clone());
    Ok(sandbox)
}

/// A sandbox to insert.
pub struct NewSandbox<'a> {
    pub cluster_id: Uuid,
    pub gateway_id: Option<Uuid>,
    pub node_id: Uuid,
    pub name: &'a str,
    pub pack_key: &'a str,
    pub image: &'a str,
    pub image_digest: Option<&'a str>,
    pub spec: &'a serde_json::Value,
    /// The lead of the stack this sandbox joins, already checked by the service.
    pub parent_id: Option<Uuid>,
    /// Who created it. Set in the statement that writes the row rather than in a
    /// second one: a member's insert has to carry its owner to pass the
    /// row-level policy (migration 0090), which only lets them write a sandbox
    /// that is theirs or handed to them.
    pub owner_user_id: Option<Uuid>,
}

/// Insert a sandbox and return it.
pub async fn insert(
    scope: &mut TenantScope<'_>,
    new: NewSandbox<'_>,
) -> Result<Sandbox, StoreError> {
    let environment_id = scope.environment_id();
    let sql = format!(
        "WITH inserted AS (
            INSERT INTO sandboxes
                (environment_id, cluster_id, gateway_id, node_id, name, pack_key,
                 image, image_digest, spec, status, health, parent_id, owner_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending', 'unknown', $10, $11)
            RETURNING *
         )
         SELECT {SANDBOX_COLUMNS}
           FROM inserted s
           LEFT JOIN gateways g ON g.id = s.gateway_id
           LEFT JOIN nodes n ON n.id = s.node_id"
    );

    let row = sqlx::query(&sql)
        .bind(environment_id)
        .bind(new.cluster_id)
        .bind(new.gateway_id)
        .bind(new.node_id)
        .bind(new.name)
        .bind(new.pack_key)
        .bind(new.image)
        .bind(new.image_digest)
        .bind(new.spec)
        .bind(new.parent_id)
        .bind(new.owner_user_id)
        .fetch_one(scope.connection())
        .await?;

    row_to_sandbox(&row)
}

/// Put a sandbox into a stack, or take it out with `None`.
///
/// The rules (one level, no self, same environment) are the service's; this
/// only writes. The environment filter still applies, so a foreign id updates
/// nothing and reads as not found.
pub async fn set_parent(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "UPDATE sandboxes SET parent_id = $3, updated_at = now()
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(parent_id)
    .execute(scope.connection())
    .await?
    .rows_affected();
    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// How many sandboxes point at this one.
pub async fn member_count(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<i64, StoreError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sandboxes WHERE environment_id = $1 AND parent_id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .fetch_one(scope.connection())
    .await?;
    Ok(count)
}

/// Change a sandbox's status, guarded by the expected version.
///
/// Returns `VersionConflict` when another writer got there first, which becomes
/// a 409 so the client can reload rather than a silent overwrite.
pub async fn update_status(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    expected_version: i64,
    status: SandboxStatus,
    health: SandboxHealth,
) -> Result<Sandbox, StoreError> {
    let environment_id = scope.environment_id();
    let sql = format!(
        "WITH updated AS (
            UPDATE sandboxes
               SET status = $3, health = $4,
                   version = version + 1, updated_at = now()
             WHERE environment_id = $1 AND id = $2 AND version = $5
         RETURNING *
         )
         SELECT {SANDBOX_COLUMNS}
           FROM updated s
           LEFT JOIN gateways g ON g.id = s.gateway_id
           LEFT JOIN nodes n ON n.id = s.node_id"
    );

    let row = sqlx::query(&sql)
        .bind(environment_id)
        .bind(id)
        .bind(status.as_str())
        .bind(health.as_str())
        .bind(expected_version)
        .fetch_optional(scope.connection())
        .await?;

    match row {
        Some(row) => row_to_sandbox(&row),
        None => {
            // Distinguish "gone" from "changed": the caller needs different
            // advice for each.
            let exists: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM sandboxes WHERE environment_id = $1 AND id = $2",
            )
            .bind(environment_id)
            .bind(id)
            .fetch_optional(scope.connection())
            .await?;
            if exists.is_some() {
                Err(StoreError::VersionConflict)
            } else {
                Err(StoreError::NotFound)
            }
        }
    }
}

/// Delete a sandbox.
pub async fn delete(scope: &mut TenantScope<'_>, id: Uuid) -> Result<(), StoreError> {
    let affected = sqlx::query(
        "DELETE FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .execute(scope.connection())
    .await?
    .rows_affected();

    if affected == 0 {
        Err(StoreError::NotFound)
    } else {
        Ok(())
    }
}

/// Count sandboxes by status, for the environment summary.
pub async fn count_by_status(
    scope: &mut TenantScope<'_>,
) -> Result<std::collections::BTreeMap<String, i64>, StoreError> {
    let rows = sqlx::query(
        "SELECT status, COUNT(*) AS n FROM sandboxes WHERE environment_id = $1 GROUP BY status",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = std::collections::BTreeMap::new();
    for row in rows {
        let status: String = row.try_get("status")?;
        let count: i64 = row.try_get("n")?;
        out.insert(status, count);
    }
    Ok(out)
}

/// Names and ids only, for a picker that must not load full rows.
pub async fn names(scope: &mut TenantScope<'_>) -> Result<Vec<(Uuid, String)>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, name FROM sandboxes WHERE environment_id = $1 ORDER BY name LIMIT 1000",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push((row.try_get("id")?, row.try_get("name")?));
    }
    Ok(out)
}

/// Timestamp helper kept next to the queries that use it, so the cursor and the
/// column agree on the type.
pub type Timestamp = DateTime<Utc>;

/// Sandbox identity plus the gateway it is placed on, for the cluster graph.
///
/// The graph needs the placement link to draw an edge; without it every sandbox
/// would float unconnected and the view would say nothing about the flow.
pub async fn topology_rows(
    scope: &mut TenantScope<'_>,
) -> Result<Vec<TopologySandbox>, StoreError> {
    let rows = sqlx::query(
        "SELECT s.id, s.name, s.gateway_id, s.status, s.node_id, n.name AS node_name
           FROM sandboxes s
           LEFT JOIN nodes n ON n.id = s.node_id
          WHERE s.environment_id = $1
          ORDER BY n.name NULLS LAST, s.name
          LIMIT 2000",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TopologySandbox {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            gateway_id: row.try_get("gateway_id")?,
            status: row.try_get("status")?,
            node_id: row.try_get("node_id")?,
            node_name: row.try_get("node_name")?,
        });
    }
    Ok(out)
}

/// One sandbox as the cluster graph needs it: where it is and which node holds
/// it.
///
/// The node is part of the answer so the console can group the graph without a
/// second placement rule of its own, and a sandbox whose node row is gone still
/// appears, with an empty node name.
#[derive(Debug, Clone)]
pub struct TopologySandbox {
    pub id: Uuid,
    pub name: String,
    pub gateway_id: Option<Uuid>,
    pub status: String,
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
}

/// The role a pack plays (`agent`, `tool`, `gateway`), or `None` for a pack that
/// does not exist.
pub async fn pack_role(
    scope: &mut TenantScope<'_>,
    pack_key: &str,
) -> Result<Option<String>, StoreError> {
    Ok(sqlx::query_scalar("SELECT role FROM agent_packs WHERE key = $1")
        .bind(pack_key)
        .fetch_optional(scope.connection())
        .await?)
}

/// The sandboxes on a node that could lead a stack: an agent-role pack, not
/// itself a member, not failed.
pub async fn lead_candidates(
    scope: &mut TenantScope<'_>,
    node_id: Uuid,
) -> Result<Vec<Uuid>, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT s.id
           FROM sandboxes s
           JOIN agent_packs p ON p.key = s.pack_key
          WHERE s.environment_id = $1 AND s.node_id = $2
            AND s.parent_id IS NULL AND p.role = 'agent'
            AND s.status NOT IN ('failed', 'deleting', 'deleted')
          ORDER BY s.created_at",
    )
    .bind(scope.environment_id())
    .bind(node_id)
    .fetch_all(scope.connection())
    .await?)
}

/// A stack: the lead first, then its members by name.
///
/// A sandbox that is nobody's lead comes back alone, which is a stack of one.
pub async fn stack_members(
    scope: &mut TenantScope<'_>,
    lead_id: Uuid,
) -> Result<Vec<Sandbox>, StoreError> {
    let sql = format!(
        "SELECT {SANDBOX_COLUMNS}
           FROM sandboxes s
           LEFT JOIN gateways g ON g.id = s.gateway_id
           LEFT JOIN nodes n ON n.id = s.node_id
          WHERE s.environment_id = $1 AND (s.id = $2 OR s.parent_id = $2)
          ORDER BY (s.id = $2) DESC, s.name"
    );
    let rows = sqlx::query(&sql)
        .bind(scope.environment_id())
        .bind(lead_id)
        .fetch_all(scope.connection())
        .await?;
    rows.iter().map(row_to_sandbox).collect()
}

/// Replace the stored spec (the desired resources and settings of a sandbox).
pub async fn update_spec(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    spec: &serde_json::Value,
) -> Result<(), StoreError> {
    let result = sqlx::query(
        "UPDATE sandboxes SET spec = $3, updated_at = now() WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .bind(spec)
    .execute(scope.connection())
    .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// Set one of a sandbox's recording mutes (B11).
///
/// `bodies == false` sets `telemetry_mute_until` (nothing is stored); `bodies == true` sets
/// `telemetry_mute_bodies_until` (flows are stored, bodies and chats are not). `None` clears it.
pub async fn set_telemetry_mute(
    scope: &mut TenantScope<'_>,
    id: Uuid,
    until: Option<DateTime<Utc>>,
    bodies: bool,
) -> Result<(), StoreError> {
    let sql = if bodies {
        "UPDATE sandboxes SET telemetry_mute_bodies_until = $3 WHERE environment_id = $1 AND id = $2"
    } else {
        "UPDATE sandboxes SET telemetry_mute_until = $3 WHERE environment_id = $1 AND id = $2"
    };
    let result = sqlx::query(sql)
        .bind(scope.environment_id())
        .bind(id)
        .bind(until)
        .execute(scope.connection())
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

/// A sandbox's recording mutes: `(mute_until, mute_bodies_until)`; a missing sandbox reads as none.
pub async fn telemetry_mute(
    scope: &mut TenantScope<'_>,
    id: Uuid,
) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>), StoreError> {
    let row = sqlx::query(
        "SELECT telemetry_mute_until, telemetry_mute_bodies_until FROM sandboxes
          WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(id)
    .fetch_optional(scope.connection())
    .await?;
    match row {
        Some(row) => Ok((
            row.try_get("telemetry_mute_until")?,
            row.try_get("telemetry_mute_bodies_until")?,
        )),
        None => Ok((None, None)),
    }
}
