//! The inventory tree: `GET /inventory/tree` (package B01).
//!
//! Named `inventory_tree` because `inventory.rs` already serves a sandbox's installed software.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::api::principal::Auth;
use crate::domain::inventory::{TreeNode, TreeView};
use crate::error::ApiError;
use crate::services::inventory_tree_service;
use crate::state::AppState;

/// The query of the tree endpoint.
#[derive(Debug, Deserialize)]
pub struct TreeQuery {
    /// `infrastructure` (default), `workloads` or `models`.
    #[serde(default)]
    pub view: Option<String>,
}

/// The tree for one view.
pub async fn tree(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<TreeQuery>,
) -> Result<Json<TreeNode>, ApiError> {
    let view = match query.view.as_deref() {
        None | Some("") => TreeView::Infrastructure,
        Some(value) => TreeView::parse(value)
            .ok_or_else(|| ApiError::validation("view must be infrastructure, workloads or models."))?,
    };
    Ok(Json(inventory_tree_service::tree(&state.services, &auth.principal, view).await?))
}
