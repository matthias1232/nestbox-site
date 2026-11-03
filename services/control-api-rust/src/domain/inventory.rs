//! The inventory tree: organisation, data center, site, node, sandbox.
//!
//! Pure shapes and one pure builder. The store fetches flat rows; `build_tree` turns
//! them into the tree the console draws (contract C3). Package B01 fills the builder.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Which grouping of the same rows the caller wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TreeView {
    /// Organisation, data center, site, node, sandbox.
    Infrastructure,
    /// Organisation, data center, user, sandbox.
    Workloads,
    /// Model, install on a node; clusters group their member installs.
    Models,
}

impl TreeView {
    /// The query-string spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Infrastructure => "infrastructure",
            Self::Workloads => "workloads",
            Self::Models => "models",
        }
    }

    /// Parse the query-string spelling.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "infrastructure" => Self::Infrastructure,
            "workloads" => Self::Workloads,
            "models" => Self::Models,
            _ => return None,
        })
    }
}

/// Roll-up numbers shown next to a tree node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TreeCounts {
    pub data_centers: i64,
    pub sites: i64,
    pub nodes: i64,
    pub sandboxes: i64,
    pub running: i64,
    pub alerts: i64,
    pub allow_all: i64,
}

/// One node of the tree. Fields that only some kinds have are optional.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TreeNode {
    /// `organization`, `data_center`, `site`, `node`, `sandbox`, `user`, `model`, `install`, `cluster`.
    pub kind: String,
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<TreeCounts>,
    /// Kind-specific extra fields (tunnel state, budget, owner, pack, egress mode...).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extra: serde_json::Value,
    pub children: Vec<TreeNode>,
}

/// One sandbox with where it lives, as the store returns it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlacementRow {
    pub environment_id: Uuid,
    pub environment_name: String,
    pub site_id: Option<Uuid>,
    pub site_name: Option<String>,
    pub site_kind: Option<String>,
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
    pub node_status: Option<String>,
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: Option<String>,
    pub sandbox_status: Option<String>,
    pub owner_id: Option<Uuid>,
    pub owner_name: Option<String>,
}

/// One model install, as the store returns it for the models view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstallRow {
    pub install_id: Uuid,
    pub model_key: String,
    pub node_id: Uuid,
    pub node_name: String,
    pub state: String,
}

/// A data center member, for the workloads view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserRow {
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
}

/// Build the tree for one view from flat rows.
///
/// Wave 0 returns an empty organisation root; package B01 builds the real tree.
pub fn build_tree(view: TreeView, _rows: Vec<PlacementRow>) -> TreeNode {
    let _ = view;
    TreeNode {
        kind: "organization".into(),
        id: Uuid::nil().to_string(),
        name: "Platform".into(),
        status: "ok".into(),
        site_kind: None,
        counts: Some(TreeCounts::default()),
        extra: serde_json::Value::Null,
        children: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_round_trips_through_its_string() {
        for view in [TreeView::Infrastructure, TreeView::Workloads, TreeView::Models] {
            assert_eq!(TreeView::parse(view.as_str()), Some(view));
        }
        assert_eq!(TreeView::parse("nonsense"), None);
    }

    #[test]
    fn the_stub_builder_returns_an_empty_root() {
        let root = build_tree(TreeView::Infrastructure, Vec::new());
        assert_eq!(root.kind, "organization");
        assert!(root.children.is_empty());
    }
}
