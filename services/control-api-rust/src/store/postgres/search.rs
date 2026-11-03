//! Search queries, one per kind.
//!
//! Every function takes the `ILIKE` pattern built by `domain::search::like_pattern`
//! (`%text%`, escaped with `\`). The id-prefix match derives `text%` from it. Rows come
//! back as hits with score 0: the service scores and sorts them.

use sqlx::Row;
use uuid::Uuid;

use super::super::TenantScope;
use crate::domain::search::{console_path, SearchHit, SearchKind};
use crate::error::StoreError;

/// The id-prefix pattern (`text%`) for a `%text%` pattern.
fn prefix_of(pattern: &str) -> String {
    let inner = pattern.strip_prefix('%').unwrap_or(pattern);
    inner.to_string()
}

/// Build a hit from the four columns every query returns.
fn hit(kind: SearchKind, environment_id: Option<Uuid>, row: &sqlx::postgres::PgRow) -> Result<SearchHit, StoreError> {
    let id: String = row.try_get("id")?;
    Ok(SearchHit {
        kind: kind.as_str().to_string(),
        path: console_path(kind, &id),
        id,
        name: row.try_get("name")?,
        subtitle: row.try_get::<Option<String>, _>("subtitle")?.unwrap_or_default(),
        status: row.try_get("status")?,
        environment_id,
        score: 0.0,
    })
}

/// Sandboxes whose name or id matches the pattern.
pub async fn sandboxes(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT s.id::text AS id, s.name, s.status,
                 concat_ws(' · ', s.pack_key, n.display_name, e.name) AS subtitle
            FROM sandboxes s
            LEFT JOIN nodes n ON n.id = s.node_id
            JOIN environments e ON e.id = s.environment_id
           WHERE s.environment_id = $1 AND s.status <> 'deleted'
             AND (s.name ILIKE $2 ESCAPE '\' OR s.id::text ILIKE $3 ESCAPE '\')
           ORDER BY s.name LIMIT $4",
    )
    .bind(environment_id)
    .bind(q)
    .bind(prefix_of(q))
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Sandbox, Some(environment_id), row)).collect()
}

/// Nodes whose name, display name or id matches the pattern.
pub async fn nodes(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT n.id::text AS id, n.display_name AS name, n.status,
                 concat_ws(' · ', n.name, n.status, st.name) AS subtitle
            FROM nodes n
            LEFT JOIN sites st ON st.id = n.site_id
           WHERE n.environment_id = $1
             AND (n.display_name ILIKE $2 ESCAPE '\' OR n.name ILIKE $2 ESCAPE '\'
                  OR n.id::text ILIKE $3 ESCAPE '\')
           ORDER BY n.display_name LIMIT $4",
    )
    .bind(environment_id)
    .bind(q)
    .bind(prefix_of(q))
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Node, Some(environment_id), row)).collect()
}

/// Catalogue models whose key or name matches the pattern (the table is global).
pub async fn models(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let rows = sqlx::query(
        r"SELECT m.key AS id, m.name, NULL::text AS status,
                 concat_ws(' · ', m.family, m.parameters) AS subtitle
            FROM models m
           WHERE (m.environment_id IS NULL OR m.environment_id = $1)
             AND (m.name ILIKE $2 ESCAPE '\' OR m.key ILIKE $2 ESCAPE '\')
           ORDER BY m.name LIMIT $3",
    )
    .bind(scope.environment_id())
    .bind(q)
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Model, None, row)).collect()
}

/// Policies whose name or id matches the pattern.
pub async fn policies(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT p.id::text AS id, p.name, NULL::text AS status, p.scope AS subtitle
            FROM policies p
           WHERE p.environment_id = $1
             AND (p.name ILIKE $2 ESCAPE '\' OR p.id::text ILIKE $3 ESCAPE '\')
           ORDER BY p.name LIMIT $4",
    )
    .bind(environment_id)
    .bind(q)
    .bind(prefix_of(q))
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Policy, Some(environment_id), row)).collect()
}

/// Users of the data center's organization whose name or email matches the pattern.
pub async fn users(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT u.id::text AS id, u.display_name AS name, u.status, u.role AS subtitle
            FROM users u
            JOIN environments e ON e.organization_id = u.organization_id
           WHERE e.id = $1
             AND (u.display_name ILIKE $2 ESCAPE '\' OR u.email ILIKE $2 ESCAPE '\')
           ORDER BY u.display_name LIMIT $3",
    )
    .bind(environment_id)
    .bind(q)
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::User, Some(environment_id), row)).collect()
}

/// Sites whose name matches the pattern.
pub async fn sites(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT s.id::text AS id, s.name, NULL::text AS status, s.kind AS subtitle
            FROM sites s
           WHERE s.environment_id = $1 AND s.name ILIKE $2 ESCAPE '\'
           ORDER BY s.name LIMIT $3",
    )
    .bind(environment_id)
    .bind(q)
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Site, Some(environment_id), row)).collect()
}

/// Model clusters whose name matches the pattern.
pub async fn clusters(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT c.id::text AS id, c.name, NULL::text AS status, c.model_key AS subtitle
            FROM model_clusters c
           WHERE c.environment_id = $1 AND c.name ILIKE $2 ESCAPE '\'
           ORDER BY c.name LIMIT $3",
    )
    .bind(environment_id)
    .bind(q)
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Cluster, Some(environment_id), row)).collect()
}

/// Jobs whose id starts with the text (kinds are not searched; ids are).
pub async fn jobs(scope: &mut TenantScope<'_>, q: &str, limit: i64) -> Result<Vec<SearchHit>, StoreError> {
    let environment_id = scope.environment_id();
    let rows = sqlx::query(
        r"SELECT j.id::text AS id, j.kind AS name, j.status, j.status AS subtitle
            FROM jobs j
           WHERE j.environment_id = $1 AND j.id::text ILIKE $2 ESCAPE '\'
           ORDER BY j.created_at DESC LIMIT $3",
    )
    .bind(environment_id)
    .bind(prefix_of(q))
    .bind(limit)
    .fetch_all(scope.connection())
    .await?;
    rows.iter().map(|row| hit(SearchKind::Job, Some(environment_id), row)).collect()
}
