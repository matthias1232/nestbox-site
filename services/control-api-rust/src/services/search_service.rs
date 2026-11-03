//! Global search over everything the caller may see.

use std::collections::HashSet;

use uuid::Uuid;

use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::search::{
    console_path, like_pattern, score, score_with_id, SearchHit, SearchKind, SearchParams,
};
use crate::error::ApiError;
use crate::store::postgres::auth as auth_store;
use crate::store::postgres::search as search_store;
use crate::store::TenantScope;

/// Search the kinds named in `params`, best hits first.
///
/// Walks every data center the caller can reach (or the one asked for), one scope each, so
/// row-level security decides what is visible. Users are searched for admins only. The
/// `status` filter applies to sandboxes and nodes.
pub async fn search(
    ctx: &ServiceContext,
    principal: &Principal,
    params: &SearchParams,
) -> Result<Vec<SearchHit>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let q = params.q.trim();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let limit = params.limit.clamp(1, 50);
    let pattern = like_pattern(q);
    let kinds: Vec<SearchKind> = if params.kinds.is_empty() {
        SearchKind::ALL.to_vec()
    } else {
        params.kinds.clone()
    };
    let environments = auth_store::environments_for_user(&ctx.pool, principal.user_id)
        .await
        .map_err(ApiError::from)?;

    let mut hits: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (environment_id, environment_name, role) in environments {
        if params.environment.is_some_and(|wanted| wanted != environment_id) {
            continue;
        }
        if kinds.contains(&SearchKind::DataCenter) {
            push(&mut hits, &mut seen, data_center_hit(environment_id, &environment_name, q));
        }
        let mut acting = principal.clone();
        acting.environment_id = environment_id;
        acting.role = role;
        let mut scope = TenantScope::begin_for(&ctx.pool, &acting)
            .await
            .map_err(ApiError::from)?;
        for kind in &kinds {
            let found = match kind {
                SearchKind::Sandbox => search_store::sandboxes(&mut scope, &pattern, limit).await,
                SearchKind::Node => search_store::nodes(&mut scope, &pattern, limit).await,
                SearchKind::Model => search_store::models(&mut scope, &pattern, limit).await,
                SearchKind::Policy => search_store::policies(&mut scope, &pattern, limit).await,
                SearchKind::User if role == Role::Admin => {
                    search_store::users(&mut scope, &pattern, limit).await
                }
                SearchKind::Site => search_store::sites(&mut scope, &pattern, limit).await,
                SearchKind::Cluster => search_store::clusters(&mut scope, &pattern, limit).await,
                SearchKind::Job => search_store::jobs(&mut scope, &pattern, limit).await,
                SearchKind::User | SearchKind::DataCenter => continue,
            }
            .map_err(ApiError::from)?;
            for mut item in found {
                if matches!(kind, SearchKind::Sandbox | SearchKind::Node) {
                    if let Some(wanted) = params.status.as_deref() {
                        if item.status.as_deref() != Some(wanted) {
                            continue;
                        }
                    }
                }
                item.score = f64::from(score_with_id(&item.name, &item.id, q));
                push(&mut hits, &mut seen, Some(item));
            }
        }
        scope.commit().await.map_err(ApiError::from)?;
    }
    hits.retain(|item| item.score > 0.0);
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    hits.truncate(usize::try_from(limit).unwrap_or(20));
    Ok(hits)
}

/// Add a hit once per (kind, id): the global model table shows up in every data center.
fn push(hits: &mut Vec<SearchHit>, seen: &mut HashSet<(String, String)>, item: Option<SearchHit>) {
    if let Some(item) = item {
        if seen.insert((item.kind.clone(), item.id.clone())) {
            hits.push(item);
        }
    }
}

/// A data center is matched by name from the reachable list; no query needed.
fn data_center_hit(environment_id: Uuid, name: &str, q: &str) -> Option<SearchHit> {
    let value = score(name, q);
    if value <= 0.0 {
        return None;
    }
    let id = environment_id.to_string();
    Some(SearchHit {
        kind: SearchKind::DataCenter.as_str().to_string(),
        path: console_path(SearchKind::DataCenter, &id),
        id,
        name: name.to_string(),
        subtitle: "data center".to_string(),
        status: None,
        environment_id: Some(environment_id),
        score: f64::from(value),
    })
}
