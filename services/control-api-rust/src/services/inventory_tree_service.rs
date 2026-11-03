//! The inventory tree the caller may see (package B01).

use std::collections::BTreeSet;

use super::{ServiceContext, node_service, require};
use crate::domain::auth::{Permission, Principal, Role};
use crate::domain::inventory::{
    ClusterRow, InstallRow, PlacementRow, TreeResponse, TreeView, UserRow, build_tree,
};
use crate::error::ApiError;
use crate::store::TenantScope;
use crate::store::postgres::auth as auth_store;
use crate::store::postgres::inventory as inventory_store;

/// The tree for one view, restricted to what the caller may see.
///
/// Reaches every data center the caller can (one scope per data center, with the caller's
/// role there, so row-level security hides what a member must not see), fetches flat rows,
/// and lets the pure builder shape them. Node budgets are included only for operators and
/// above; no runner is probed.
pub async fn tree(
    ctx: &ServiceContext,
    principal: &Principal,
    view: TreeView,
) -> Result<TreeResponse, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let reachable = auth_store::environments_for_user(&ctx.pool, principal.user_id)
        .await
        .map_err(ApiError::from)?;
    let environment_ids: Vec<_> = reachable.iter().map(|(id, _, _)| *id).collect();
    let envs = inventory_store::environment_rows(&ctx.pool, &environment_ids)
        .await
        .map_err(ApiError::from)?;

    let mut placements: Vec<PlacementRow> = Vec::new();
    let mut users: Vec<UserRow> = Vec::new();
    let mut installs: Vec<InstallRow> = Vec::new();
    let mut clusters: Vec<ClusterRow> = Vec::new();

    for (environment_id, _, role) in &reachable {
        let scoped = Principal {
            environment_id: *environment_id,
            role: *role,
            ..principal.clone()
        };
        let mut scope = TenantScope::begin_for(&ctx.pool, &scoped)
            .await
            .map_err(ApiError::from)?;
        match view {
            TreeView::Infrastructure | TreeView::Workloads => {
                let mut rows = inventory_store::placement_rows(&mut scope)
                    .await
                    .map_err(ApiError::from)?;
                if *role >= Role::Operator {
                    attach_budgets(&mut scope, &mut rows).await?;
                }
                placements.extend(rows);
                if view == TreeView::Workloads {
                    let mut people = inventory_store::user_rows(&mut scope)
                        .await
                        .map_err(ApiError::from)?;
                    if *role < Role::Operator {
                        people.retain(|user| user.user_id == principal.user_id);
                    }
                    users.extend(people);
                }
            }
            TreeView::Models => {
                installs.extend(
                    inventory_store::install_rows(&mut scope, None)
                        .await
                        .map_err(ApiError::from)?,
                );
                clusters.extend(
                    inventory_store::cluster_rows(&mut scope)
                        .await
                        .map_err(ApiError::from)?,
                );
            }
        }
        scope.commit().await.map_err(ApiError::from)?;
    }

    let root = build_tree(view, &envs, &placements, &users, &installs, &clusters);
    Ok(TreeResponse {
        view: view.as_str().to_string(),
        generated_at: chrono::Utc::now(),
        root,
    })
}

/// Fill the free memory of every node in the rows, once per node, from the node budget.
async fn attach_budgets(
    scope: &mut TenantScope<'_>,
    rows: &mut [PlacementRow],
) -> Result<(), ApiError> {
    let node_ids: BTreeSet<_> = rows.iter().filter_map(|row| row.node_id).collect();
    let mut budgets = std::collections::BTreeMap::new();
    for node_id in node_ids {
        let budget = node_service::free_budget(scope, node_id).await?;
        budgets.insert(node_id, (budget.ram_free_bytes, budget.vram_free_bytes));
    }
    for row in rows.iter_mut() {
        if let Some((ram, vram)) = row.node_id.and_then(|id| budgets.get(&id)) {
            row.ram_free = Some(*ram);
            row.vram_free = Some(*vram);
        }
    }
    Ok(())
}
