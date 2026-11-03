//! Keeping every node's images in step with one registry (ADR 0048).
//!
//! The control plane runs the registry. An administrator assigns an image to a scope (a data
//! center, a site or one node); a loop makes each joined node hold what it was assigned, and
//! records what it found, so the console can draw images x nodes without asking every machine on
//! every page load.
//!
//! What it does not do: build images (the builder does), copy anything between nodes, or remove
//! an image from a node that no longer needs it. Removal is deliberate: a pull is cheap to repeat
//! and an image a sandbox still uses must never disappear because an assignment was withdrawn.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{require, ServiceContext};
use crate::config::Config;
use crate::domain::auth::{Permission, Principal};
use crate::error::{ApiError, ErrorCode};
use crate::runtime::router::RuntimeRouter;
use crate::store::TenantScope;

fn db(error: sqlx::Error) -> ApiError {
    ApiError::from(crate::store::map_sqlx_error(error))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Assignment {
    pub id: Uuid,
    pub image: String,
    /// `environment`, `site` or `node`.
    pub scope_type: String,
    pub scope_id: Option<Uuid>,
    pub scope_name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MatrixNode {
    pub id: Uuid,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MatrixCell {
    pub image: String,
    pub node_id: Uuid,
    /// `present`, `pulling`, `missing` or `failed`; absent when the node was never asked.
    pub state: String,
    pub error: Option<String>,
    pub size_bytes: Option<i64>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Matrix {
    pub images: Vec<String>,
    pub nodes: Vec<MatrixNode>,
    pub cells: Vec<MatrixCell>,
    pub assignments: Vec<Assignment>,
}

/// Whether a string can be an image reference: what the database accepts, and no scheme.
pub fn valid_image(image: &str) -> bool {
    let bytes = image.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 255
        && bytes[0].is_ascii_alphanumeric()
        && !image.contains("://")
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || b"._/:@-".contains(b))
}

/// The images a node should hold: what was assigned to its data center, its site or itself.
pub fn wanted_for(node: Uuid, site: Option<Uuid>, assignments: &[(String, String, Option<Uuid>)]) -> BTreeSet<String> {
    assignments
        .iter()
        .filter(|(_, scope, id)| match scope.as_str() {
            "environment" => true,
            "site" => site.is_some() && *id == site,
            "node" => *id == Some(node),
            _ => false,
        })
        .map(|(image, _, _)| image.clone())
        .collect()
}

/// The repositories and tags in the registry, as `name:tag`.
pub async fn registry_images(config: &Config) -> Result<Vec<String>, ApiError> {
    let base = format!("http://{}", config.overlay.registry);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let unavailable = || ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, "The image registry does not answer.");
    let catalog: serde_json::Value = http
        .get(format!("{base}/v2/_catalog"))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| unavailable())?
        .json()
        .await
        .map_err(|_| unavailable())?;
    let mut out = Vec::new();
    for repo in catalog["repositories"].as_array().cloned().unwrap_or_default() {
        let Some(repo) = repo.as_str() else { continue };
        let tags: serde_json::Value = match http.get(format!("{base}/v2/{repo}/tags/list")).send().await {
            Ok(response) => response.json().await.unwrap_or_default(),
            Err(_) => continue,
        };
        for tag in tags["tags"].as_array().cloned().unwrap_or_default() {
            if let Some(tag) = tag.as_str() {
                out.push(format!("{repo}:{tag}"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Assign an image to a scope. The same assignment twice is one assignment.
pub async fn assign(
    ctx: &ServiceContext,
    principal: &Principal,
    image: &str,
    scope_type: &str,
    scope_id: Option<Uuid>,
) -> Result<Assignment, ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let image = image.trim();
    if !valid_image(image) {
        return Err(ApiError::validation("That is not an image name, for example nestbox-claude-code:local."));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let scope_name = match (scope_type, scope_id) {
        ("environment", None) => "the whole data center".to_string(),
        ("site", Some(id)) => sqlx::query_scalar::<_, String>("SELECT name FROM sites WHERE id = $1 AND environment_id = $2")
            .bind(id)
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await
            .map_err(db)?
            .ok_or_else(|| ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "site"))?,
        ("node", Some(id)) => sqlx::query_scalar::<_, String>("SELECT name FROM nodes WHERE id = $1 AND environment_id = $2")
            .bind(id)
            .bind(scope.environment_id())
            .fetch_optional(scope.connection())
            .await
            .map_err(db)?
            .ok_or_else(|| ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "node"))?,
        _ => return Err(ApiError::validation("An assignment is for the data center, a site or a node.")),
    };
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO image_assignments (environment_id, image, scope_type, scope_id, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (environment_id, image, scope_type, coalesce(scope_id, '00000000-0000-0000-0000-000000000000'::uuid))
         DO UPDATE SET image = EXCLUDED.image RETURNING id",
    )
    .bind(scope.environment_id())
    .bind(image)
    .bind(scope_type)
    .bind(scope_id)
    .bind(principal.user_id)
    .fetch_one(scope.connection())
    .await
    .map_err(db)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "image.assign",
            subject_type: Some("image"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "image": image, "scope": scope_type, "target": scope_name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(Assignment { id, image: image.to_string(), scope_type: scope_type.to_string(), scope_id, scope_name })
}

/// Withdraw an assignment. The image stays on the nodes that already hold it.
pub async fn unassign(ctx: &ServiceContext, principal: &Principal, id: Uuid) -> Result<(), ApiError> {
    require(principal, Permission::EnvironmentManage)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let done = sqlx::query("DELETE FROM image_assignments WHERE id = $1 AND environment_id = $2")
        .bind(id)
        .bind(scope.environment_id())
        .execute(scope.connection())
        .await
        .map_err(db)?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found(ErrorCode::NODE_NOT_FOUND, "assignment"));
    }
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// Images x nodes for this data center, from what the nodes last reported.
pub async fn matrix(ctx: &ServiceContext, principal: &Principal) -> Result<Matrix, ApiError> {
    require(principal, Permission::NodeRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal).await.map_err(ApiError::from)?;
    let nodes: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, name, status FROM nodes WHERE environment_id = $1 AND wg_public_key IS NOT NULL ORDER BY name",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    let assignments: Vec<(Uuid, String, String, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT a.id, a.image, a.scope_type, a.scope_id,
                CASE a.scope_type WHEN 'environment' THEN 'the whole data center'
                                  WHEN 'site' THEN coalesce((SELECT name FROM sites WHERE id = a.scope_id), '?')
                                  ELSE coalesce((SELECT name FROM nodes WHERE id = a.scope_id), '?') END
           FROM image_assignments a WHERE a.environment_id = $1 ORDER BY a.image, a.scope_type",
    )
    .bind(scope.environment_id())
    .fetch_all(scope.connection())
    .await
    .map_err(db)?;
    scope.commit().await.map_err(ApiError::from)?;
    let ids: Vec<Uuid> = nodes.iter().map(|n| n.0).collect();
    let cells: Vec<(String, Uuid, String, Option<String>, Option<i64>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT image, node_id, state, error, size_bytes, checked_at FROM node_images WHERE node_id = ANY($1) ORDER BY image",
    )
    .bind(&ids)
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    let mut images: BTreeSet<String> = assignments.iter().map(|a| a.1.clone()).collect();
    images.extend(cells.iter().map(|c| c.0.clone()));
    Ok(Matrix {
        images: images.into_iter().collect(),
        nodes: nodes.into_iter().map(|(id, name, status)| MatrixNode { id, name, status }).collect(),
        cells: cells
            .into_iter()
            .map(|(image, node_id, state, error, size_bytes, checked_at)| MatrixCell { image, node_id, state, error, size_bytes, checked_at })
            .collect(),
        assignments: assignments
            .into_iter()
            .map(|(id, image, scope_type, scope_id, scope_name)| Assignment { id, image, scope_type, scope_id, scope_name })
            .collect(),
    })
}

/// One pass over every joined node: make it hold what it was assigned, and record what it holds.
///
/// A node that does not answer keeps its last record (stale, with the time it was checked); a
/// pull that fails is recorded with the runner's reason and tried again next pass.
pub async fn reconcile(ctx: &ServiceContext, router: &RuntimeRouter) -> Result<usize, ApiError> {
    let nodes: Vec<(Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, environment_id, site_id FROM nodes WHERE wg_public_key IS NOT NULL AND status = 'online'",
    )
    .fetch_all(&ctx.pool)
    .await
    .map_err(db)?;
    let mut pulled = 0;
    for (node_id, environment_id, site_id) in nodes {
        let assignments: Vec<(String, String, Option<Uuid>)> = {
            let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
            let rows = sqlx::query_as("SELECT image, scope_type, scope_id FROM image_assignments")
                .fetch_all(scope.connection())
                .await
                .map_err(db)?;
            scope.commit().await.map_err(ApiError::from)?;
            rows
        };
        let wanted = wanted_for(node_id, site_id, &assignments);
        let adapter = router.for_node(Some(node_id));
        let held: BTreeMap<String, (i64, String)> = match tokio::time::timeout(std::time::Duration::from_secs(10), adapter.images()).await {
            Ok(Ok(list)) => list.into_iter().map(|i| (i.image, (i.size, i.id))).collect(),
            _ => continue,
        };
        for (image, (size, _)) in &held {
            record(ctx, node_id, image, "present", Some(*size), None).await?;
        }
        for image in wanted {
            if held.contains_key(&image) {
                continue;
            }
            record(ctx, node_id, &image, "pulling", None, None).await?;
            match tokio::time::timeout(std::time::Duration::from_secs(900), adapter.pull_image(&image)).await {
                Ok(Ok(fact)) => {
                    pulled += 1;
                    record(ctx, node_id, &image, "present", Some(fact.size), None).await?;
                }
                Ok(Err(error)) => record(ctx, node_id, &image, "failed", None, Some(&error.to_string())).await?,
                Err(_) => record(ctx, node_id, &image, "failed", None, Some("the pull took longer than 15 minutes")).await?,
            }
        }
    }
    Ok(pulled)
}

async fn record(ctx: &ServiceContext, node_id: Uuid, image: &str, state: &str, size: Option<i64>, error: Option<&str>) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO node_images (node_id, image, state, size_bytes, error, checked_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (node_id, image) DO UPDATE
            SET state = EXCLUDED.state, size_bytes = coalesce(EXCLUDED.size_bytes, node_images.size_bytes),
                error = EXCLUDED.error, checked_at = now()",
    )
    .bind(node_id)
    .bind(image)
    .bind(state)
    .bind(size)
    .bind(error.map(|e| e.chars().take(400).collect::<String>()))
    .execute(&ctx.pool)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_name_is_checked_like_the_database_checks_it() {
        assert!(valid_image("nestbox-claude-code:local"));
        assert!(valid_image("10.77.0.1:5000/tool@sha256:abc"));
        assert!(!valid_image(""));
        assert!(!valid_image("-rf"));
        assert!(!valid_image("http://evil/x"));
        assert!(!valid_image("a b"));
    }

    #[test]
    fn a_node_wants_what_its_data_center_its_site_and_itself_were_given() {
        let node = Uuid::new_v4();
        let site = Uuid::new_v4();
        let rows = vec![
            ("all:1".to_string(), "environment".to_string(), None),
            ("site:1".to_string(), "site".to_string(), Some(site)),
            ("other-site:1".to_string(), "site".to_string(), Some(Uuid::new_v4())),
            ("mine:1".to_string(), "node".to_string(), Some(node)),
            ("other-node:1".to_string(), "node".to_string(), Some(Uuid::new_v4())),
        ];
        let wanted = wanted_for(node, Some(site), &rows);
        assert_eq!(wanted.into_iter().collect::<Vec<_>>(), vec!["all:1", "mine:1", "site:1"]);
        // A node with no site gets no site assignments, not all of them.
        assert_eq!(wanted_for(node, None, &rows).len(), 2);
    }
}
