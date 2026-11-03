//! The inventory tree: organisation, data center, site, node, sandbox.
//!
//! Pure shapes and one pure builder. The store fetches flat rows; `build_tree` turns
//! them into the tree the console draws (contract C3).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
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

/// The most sandboxes one node lists before the tree marks it `truncated`.
pub const MAX_SANDBOXES_PER_NODE: usize = 400;

/// Roll-up numbers shown next to a tree node: everything below it, not the node itself.
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

impl TreeCounts {
    /// Add another count to this one.
    pub fn add(&mut self, other: &TreeCounts) {
        self.data_centers += other.data_centers;
        self.sites += other.sites;
        self.nodes += other.nodes;
        self.sandboxes += other.sandboxes;
        self.running += other.running;
        self.alerts += other.alerts;
        self.allow_all += other.allow_all;
    }
}

/// One node of the tree. Fields that only some kinds have sit flat next to these.
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
    /// Kind-specific fields (tunnel state, budget, owner, pack, egress mode...), flattened
    /// into the node's JSON object.
    #[serde(default, flatten)]
    #[schema(value_type = Object)]
    pub extra: serde_json::Map<String, serde_json::Value>,
    pub children: Vec<TreeNode>,
}

/// The answer of `GET /inventory/tree` (contract C3).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TreeResponse {
    /// The view that was built.
    pub view: String,
    /// When the tree was built.
    pub generated_at: DateTime<Utc>,
    /// The root of the tree.
    pub root: TreeNode,
}

/// A data center the caller reaches, with the organisation it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentRow {
    pub environment_id: Uuid,
    pub environment_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub organization_parent: Option<Uuid>,
    pub organization_kind: String,
}

/// One site-node-sandbox line as the store returns it. A site without nodes is one row
/// with the node fields `None`; a node without sandboxes one row with the sandbox fields `None`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PlacementRow {
    pub environment_id: Uuid,
    pub site_id: Option<Uuid>,
    pub site_name: Option<String>,
    pub site_kind: Option<String>,
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
    pub node_status: Option<String>,
    pub tunnel_state: Option<String>,
    pub gpu: bool,
    /// Free memory of the node; `None` when the caller may not see budgets.
    pub ram_free: Option<i64>,
    /// Free video memory of the node; `None` when the caller may not see budgets.
    pub vram_free: Option<i64>,
    pub sandbox_id: Option<Uuid>,
    pub sandbox_name: Option<String>,
    pub sandbox_status: Option<String>,
    pub pack_key: Option<String>,
    pub owner_id: Option<Uuid>,
    pub owner_name: Option<String>,
    pub egress_mode: Option<String>,
    pub allow_all: bool,
    /// Blocked connections of the sandbox in the last 15 minutes.
    pub alerts: i64,
}

/// One model install, as the store returns it for the models view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstallRow {
    pub install_id: Uuid,
    pub environment_id: Uuid,
    pub model_key: String,
    pub model_name: String,
    pub model_family: String,
    pub node_id: Uuid,
    pub node_name: String,
    pub state: String,
    pub provider_status: Option<String>,
    /// The cluster this install is a member of, if any.
    pub cluster_id: Option<Uuid>,
}

/// A model cluster, for the models view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ClusterRow {
    pub cluster_id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub model_key: String,
    pub strategy: String,
}

/// A data center member, for the workloads view.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserRow {
    pub environment_id: Uuid,
    pub user_id: Uuid,
    pub display_name: String,
    pub role: String,
}

fn key(name: &str, id: impl ToString) -> (String, String) {
    (name.to_lowercase(), id.to_string())
}

fn rank_status(rank: u8) -> &'static str {
    match rank {
        0 => "ok",
        1 => "warn",
        _ => "bad",
    }
}

/// How worrying a node and everything below it is: 0 ok, 1 warn, 2 bad.
fn health(node: &TreeNode) -> u8 {
    let own = match node.kind.as_str() {
        "sandbox" => {
            if node.status == "failed" {
                2
            } else if node.extra.get("allow_all").and_then(|v| v.as_bool()).unwrap_or(false) {
                1
            } else {
                0
            }
        }
        "node" => match node.status.as_str() {
            "offline" => 2,
            "degraded" => 1,
            _ => 0,
        },
        "install" => match node.status.as_str() {
            "failed" => 2,
            "installing" | "updating" | "uninstalling" => 1,
            _ => match node.extra.get("provider_status").and_then(|v| v.as_str()) {
                Some("unreachable") => 1,
                _ => 0,
            },
        },
        _ => 0,
    };
    node.children.iter().map(health).fold(own, u8::max)
}

/// The numbers a parent adds for one child: the child itself plus everything below it.
fn contribution(child: &TreeNode) -> TreeCounts {
    let mut counts = child.counts.clone().unwrap_or_default();
    match child.kind.as_str() {
        "data_center" => counts.data_centers += 1,
        "site" => counts.sites += 1,
        "node" => counts.nodes += 1,
        "sandbox" => {
            counts.sandboxes += 1;
            if child.status == "running" {
                counts.running += 1;
            }
            counts.alerts += child.extra.get("alerts").and_then(|v| v.as_i64()).unwrap_or(0);
            if child.extra.get("allow_all").and_then(|v| v.as_bool()).unwrap_or(false) {
                counts.allow_all += 1;
            }
        }
        _ => {}
    }
    counts
}

/// Fill `counts` from the children and, for containers, `status` from the worst child.
fn finish_container(node: &mut TreeNode) {
    let mut counts = TreeCounts::default();
    for child in &node.children {
        counts.add(&contribution(child));
    }
    node.counts = Some(counts);
    let worst = node.children.iter().map(health).max().unwrap_or(0);
    node.status = rank_status(worst).to_string();
}

fn container(kind: &str, id: String, name: &str, children: Vec<TreeNode>) -> TreeNode {
    let mut node = TreeNode {
        kind: kind.to_string(),
        id,
        name: name.to_string(),
        status: "ok".to_string(),
        site_kind: None,
        counts: None,
        extra: serde_json::Map::new(),
        children,
    };
    finish_container(&mut node);
    node
}

fn sandbox_node(row: &PlacementRow) -> Option<TreeNode> {
    let id = row.sandbox_id?;
    let mut extra = serde_json::Map::new();
    extra.insert("owner".into(), row.owner_name.clone().map_or(serde_json::Value::Null, Into::into));
    extra.insert("pack_key".into(), row.pack_key.clone().map_or(serde_json::Value::Null, Into::into));
    extra.insert("egress_mode".into(), row.egress_mode.clone().map_or(serde_json::Value::Null, Into::into));
    extra.insert("allow_all".into(), row.allow_all.into());
    extra.insert("alerts".into(), row.alerts.into());
    Some(TreeNode {
        kind: "sandbox".into(),
        id: id.to_string(),
        name: row.sandbox_name.clone().unwrap_or_default(),
        status: row.sandbox_status.clone().unwrap_or_else(|| "unknown".into()),
        site_kind: None,
        counts: None,
        extra,
        children: Vec::new(),
    })
}

/// One site-and-below accumulator for the infrastructure view.
struct SiteAcc<'a> {
    id: Uuid,
    name: &'a str,
    kind: Option<&'a str>,
    nodes: BTreeMap<(String, String), NodeAcc<'a>>,
}

struct NodeAcc<'a> {
    row: &'a PlacementRow,
    sandboxes: BTreeMap<(String, String), &'a PlacementRow>,
}

fn infrastructure_children(rows: &[&PlacementRow]) -> Vec<TreeNode> {
    let mut sites: BTreeMap<(String, String), SiteAcc> = BTreeMap::new();
    for row in rows {
        let (Some(site_id), Some(site_name)) = (row.site_id, row.site_name.as_deref()) else {
            continue;
        };
        let site = sites.entry(key(site_name, site_id)).or_insert_with(|| SiteAcc {
            id: site_id,
            name: site_name,
            kind: row.site_kind.as_deref(),
            nodes: BTreeMap::new(),
        });
        let (Some(node_id), Some(node_name)) = (row.node_id, row.node_name.as_deref()) else {
            continue;
        };
        let node = site
            .nodes
            .entry(key(node_name, node_id))
            .or_insert_with(|| NodeAcc { row, sandboxes: BTreeMap::new() });
        if let (Some(sandbox_id), Some(sandbox_name)) = (row.sandbox_id, row.sandbox_name.as_deref()) {
            node.sandboxes.insert(key(sandbox_name, sandbox_id), row);
        }
    }
    sites
        .into_values()
        .map(|site| {
            let nodes: Vec<TreeNode> = site.nodes.into_values().map(node_from).collect();
            let mut node = container("site", site.id.to_string(), site.name, nodes);
            node.site_kind = site.kind.map(str::to_string);
            node
        })
        .collect()
}

fn node_from(acc: NodeAcc) -> TreeNode {
    let row = acc.row;
    let total = acc.sandboxes.len();
    let children: Vec<TreeNode> = acc
        .sandboxes
        .values()
        .take(MAX_SANDBOXES_PER_NODE)
        .filter_map(|r| sandbox_node(r))
        .collect();
    let mut extra = serde_json::Map::new();
    extra.insert("tunnel_state".into(), row.tunnel_state.clone().map_or(serde_json::Value::Null, Into::into));
    extra.insert("gpu".into(), row.gpu.into());
    let budget = match (row.ram_free, row.vram_free) {
        (Some(ram), Some(vram)) => serde_json::json!({ "ram_free_bytes": ram, "vram_free_bytes": vram }),
        _ => serde_json::Value::Null,
    };
    extra.insert("budget".into(), budget);
    if total > MAX_SANDBOXES_PER_NODE {
        extra.insert("truncated".into(), true.into());
    }
    let mut node = container(
        "node",
        row.node_id.map(|id| id.to_string()).unwrap_or_default(),
        row.node_name.as_deref().unwrap_or(""),
        children,
    );
    // A node reports its own state, not the worst of its children.
    node.status = row.node_status.clone().unwrap_or_else(|| "unknown".into());
    node.extra = extra;
    node
}

fn workloads_children(env_id: Uuid, rows: &[&PlacementRow], users: &[UserRow]) -> Vec<TreeNode> {
    struct UserAcc {
        id: String,
        name: String,
        role: Option<String>,
        sandboxes: BTreeMap<(String, String), TreeNode>,
    }
    let shared = key("Shared", "shared");
    let mut owners: BTreeMap<(String, String), UserAcc> = BTreeMap::new();
    for user in users.iter().filter(|u| u.environment_id == env_id) {
        owners.insert(
            key(&user.display_name, user.user_id),
            UserAcc {
                id: user.user_id.to_string(),
                name: user.display_name.clone(),
                role: Some(user.role.clone()),
                sandboxes: BTreeMap::new(),
            },
        );
    }
    for row in rows {
        let Some(sandbox) = sandbox_node(row) else { continue };
        let (k, id, name) = match (row.owner_id, row.owner_name.as_deref()) {
            (Some(owner_id), Some(name)) => (key(name, owner_id), owner_id.to_string(), name.to_string()),
            _ => (shared.clone(), "shared".to_string(), "Shared".to_string()),
        };
        let acc = owners.entry(k).or_insert_with(|| UserAcc { id, name, role: None, sandboxes: BTreeMap::new() });
        acc.sandboxes.insert(key(&sandbox.name, &sandbox.id), sandbox);
    }
    owners
        .into_values()
        .map(|acc| {
            let mut node = container("user", acc.id, &acc.name, acc.sandboxes.into_values().collect());
            if let Some(role) = acc.role {
                node.extra.insert("role".into(), role.into());
            }
            node
        })
        .collect()
}

fn install_node(row: &InstallRow) -> TreeNode {
    let mut extra = serde_json::Map::new();
    extra.insert("node_id".into(), row.node_id.to_string().into());
    extra.insert("state".into(), row.state.clone().into());
    extra.insert("model_key".into(), row.model_key.clone().into());
    extra.insert("provider_status".into(), row.provider_status.clone().map_or(serde_json::Value::Null, Into::into));
    TreeNode {
        kind: "install".into(),
        id: row.install_id.to_string(),
        name: row.node_name.clone(),
        status: row.state.clone(),
        site_kind: None,
        counts: None,
        extra,
        children: Vec::new(),
    }
}

fn models_children(installs: &[InstallRow], clusters: &[ClusterRow]) -> Vec<TreeNode> {
    struct ModelAcc<'a> {
        name: &'a str,
        family: &'a str,
        loose: BTreeMap<(String, String), TreeNode>,
        clustered: BTreeMap<Uuid, BTreeMap<(String, String), TreeNode>>,
        total: usize,
    }
    let mut models: BTreeMap<(String, String), ModelAcc> = BTreeMap::new();
    for row in installs {
        let acc = models.entry(key(&row.model_name, &row.model_key)).or_insert_with(|| ModelAcc {
            name: &row.model_name,
            family: &row.model_family,
            loose: BTreeMap::new(),
            clustered: BTreeMap::new(),
            total: 0,
        });
        acc.total += 1;
        let node = install_node(row);
        let k = key(&node.name, row.install_id);
        match row.cluster_id {
            Some(cluster_id) => {
                acc.clustered.entry(cluster_id).or_default().insert(k, node);
            }
            None => {
                acc.loose.insert(k, node);
            }
        }
    }
    let mut out = Vec::new();
    for ((_, model_key), acc) in models {
        let mut by_name: BTreeMap<(String, String), TreeNode> = BTreeMap::new();
        for (cluster_id, members) in acc.clustered {
            let Some(cluster) = clusters.iter().find(|c| c.cluster_id == cluster_id) else {
                // The cluster row is not visible: show its members as plain installs.
                for (k, member) in members {
                    by_name.insert(k, member);
                }
                continue;
            };
            let mut node = container("cluster", cluster_id.to_string(), &cluster.name, members.into_values().collect());
            node.counts = None;
            node.extra.insert("strategy".into(), cluster.strategy.clone().into());
            node.extra.insert("model_key".into(), cluster.model_key.clone().into());
            by_name.insert(key(&cluster.name, cluster_id), node);
        }
        for (k, install) in acc.loose {
            by_name.insert(k, install);
        }
        let mut node = container("model", model_key, acc.name, by_name.into_values().collect());
        node.counts = None;
        node.extra.insert("family".into(), acc.family.to_string().into());
        node.extra.insert("installs".into(), (acc.total as i64).into());
        out.push(node);
    }
    out
}

/// Nest the data centers by organisation and return the root.
///
/// An organisation whose parent the caller does not reach becomes a top; one top is the
/// root, several (or none) get a synthetic `Platform` root.
fn organization_root(envs: &[EnvironmentRow], mut data_centers: BTreeMap<Uuid, TreeNode>) -> TreeNode {
    let mut orgs: BTreeMap<Uuid, (&EnvironmentRow, Vec<TreeNode>)> = BTreeMap::new();
    for env in envs {
        let entry = orgs.entry(env.organization_id).or_insert_with(|| (env, Vec::new()));
        if let Some(dc) = data_centers.remove(&env.environment_id) {
            entry.1.push(dc);
        }
    }
    fn build(id: Uuid, orgs: &BTreeMap<Uuid, (&EnvironmentRow, Vec<TreeNode>)>) -> TreeNode {
        let (env, dcs) = &orgs[&id];
        let mut children: Vec<TreeNode> = Vec::new();
        for (child_id, (child_env, _)) in orgs {
            if child_env.organization_parent == Some(id) && *child_id != id {
                children.push(build(*child_id, orgs));
            }
        }
        children.extend(dcs.iter().cloned());
        children.sort_by_key(|c| (c.kind != "organization", key(&c.name, &c.id)));
        let mut node = container("organization", id.to_string(), &env.organization_name, children);
        node.extra.insert("organization_kind".into(), env.organization_kind.clone().into());
        node
    }
    let tops: Vec<Uuid> = orgs
        .iter()
        .filter(|(id, (env, _))| match env.organization_parent {
            Some(parent) => parent == **id || !orgs.contains_key(&parent),
            None => true,
        })
        .map(|(id, _)| *id)
        .collect();
    let mut top_nodes: Vec<TreeNode> = tops.iter().map(|id| build(*id, &orgs)).collect();
    if top_nodes.len() == 1 {
        return top_nodes.remove(0);
    }
    top_nodes.sort_by_key(|c| key(&c.name, &c.id));
    container("organization", Uuid::nil().to_string(), "Platform", top_nodes)
}

/// Build the tree for one view from flat rows.
///
/// The infrastructure and workloads views nest data centers under their organisations; the
/// models view lists models (with their installs and clusters) under the root organisation.
pub fn build_tree(
    view: TreeView,
    envs: &[EnvironmentRow],
    rows: &[PlacementRow],
    users: &[UserRow],
    installs: &[InstallRow],
    clusters: &[ClusterRow],
) -> TreeNode {
    if view == TreeView::Models {
        let mut root = organization_root(envs, BTreeMap::new());
        root.children = models_children(installs, clusters);
        root.counts = Some(TreeCounts { data_centers: envs.len() as i64, ..TreeCounts::default() });
        let worst = root.children.iter().map(health).max().unwrap_or(0);
        root.status = rank_status(worst).to_string();
        return root;
    }
    let mut data_centers = BTreeMap::new();
    for env in envs {
        let env_rows: Vec<&PlacementRow> = rows.iter().filter(|r| r.environment_id == env.environment_id).collect();
        let children = match view {
            TreeView::Workloads => workloads_children(env.environment_id, &env_rows, users),
            _ => infrastructure_children(&env_rows),
        };
        data_centers.insert(
            env.environment_id,
            container("data_center", env.environment_id.to_string(), &env.environment_name, children),
        );
    }
    organization_root(envs, data_centers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> EnvironmentRow {
        EnvironmentRow {
            environment_id: Uuid::new_v4(),
            environment_name: name.into(),
            organization_id: Uuid::new_v4(),
            organization_name: "Acme".into(),
            organization_parent: None,
            organization_kind: "customer".into(),
        }
    }

    fn row(env: &EnvironmentRow, site: (Uuid, &str), node: Option<(Uuid, &str, &str)>, sandbox: Option<(&str, &str)>) -> PlacementRow {
        PlacementRow {
            environment_id: env.environment_id,
            site_id: Some(site.0),
            site_name: Some(site.1.into()),
            site_kind: Some("home".into()),
            node_id: node.map(|n| n.0),
            node_name: node.map(|n| n.1.into()),
            node_status: node.map(|n| n.2.into()),
            tunnel_state: Some("up".into()),
            gpu: true,
            ram_free: Some(1),
            vram_free: Some(2),
            sandbox_id: sandbox.map(|_| Uuid::new_v4()),
            sandbox_name: sandbox.map(|s| s.0.into()),
            sandbox_status: sandbox.map(|s| s.1.into()),
            pack_key: Some("ubuntu-dev".into()),
            owner_id: None,
            owner_name: None,
            egress_mode: Some("forward_proxy".into()),
            allow_all: false,
            alerts: 0,
        }
    }

    #[test]
    fn a_view_round_trips_through_its_string() {
        for view in [TreeView::Infrastructure, TreeView::Workloads, TreeView::Models] {
            assert_eq!(TreeView::parse(view.as_str()), Some(view));
        }
        assert_eq!(TreeView::parse("nonsense"), None);
    }

    #[test]
    fn empty_input_gives_a_root_with_zero_counts() {
        let root = build_tree(TreeView::Infrastructure, &[], &[], &[], &[], &[]);
        assert_eq!(root.kind, "organization");
        assert_eq!(root.counts, Some(TreeCounts::default()));
        assert!(root.children.is_empty());
        assert_eq!(root.status, "ok");
    }

    #[test]
    fn two_sandboxes_on_one_node_give_one_node_child() {
        let e = env("Berlin");
        let site = (Uuid::new_v4(), "main");
        let node = (Uuid::new_v4(), "gpu-07", "online");
        let rows = vec![
            row(&e, site, Some(node), Some(("b", "running"))),
            row(&e, site, Some(node), Some(("a", "stopped"))),
        ];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &rows, &[], &[], &[]);
        assert_eq!(root.name, "Acme");
        let dc = &root.children[0];
        assert_eq!(dc.kind, "data_center");
        let site_node = &dc.children[0];
        assert_eq!(site_node.site_kind.as_deref(), Some("home"));
        assert_eq!(site_node.children.len(), 1);
        let node_node = &site_node.children[0];
        assert_eq!(node_node.children.len(), 2);
        assert_eq!(node_node.children[0].name, "a");
        let counts = root.counts.clone().unwrap_or_default();
        assert_eq!((counts.data_centers, counts.sites, counts.nodes, counts.sandboxes, counts.running), (1, 1, 1, 2, 1));
    }

    #[test]
    fn a_node_without_sandboxes_and_a_site_without_nodes_still_appear() {
        let e = env("Berlin");
        let rows = vec![
            row(&e, (Uuid::new_v4(), "empty"), None, None),
            row(&e, (Uuid::new_v4(), "main"), Some((Uuid::new_v4(), "gpu-01", "online")), None),
        ];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &rows, &[], &[], &[]);
        let sites = &root.children[0].children;
        assert_eq!(sites.len(), 2);
        assert!(sites[0].children.is_empty());
        assert_eq!(sites[1].children[0].children.len(), 0);
    }

    #[test]
    fn an_offline_node_or_a_failed_sandbox_turns_the_parents_bad() {
        let e = env("Berlin");
        let site = (Uuid::new_v4(), "main");
        let offline = vec![row(&e, site, Some((Uuid::new_v4(), "n", "offline")), None)];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &offline, &[], &[], &[]);
        assert_eq!(root.status, "bad");
        let failed = vec![row(&e, site, Some((Uuid::new_v4(), "n", "online")), Some(("s", "failed")))];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &failed, &[], &[], &[]);
        assert_eq!(root.children[0].children[0].status, "bad");
    }

    #[test]
    fn allow_all_or_a_degraded_node_warns_and_alerts_roll_up() {
        let e = env("Berlin");
        let site = (Uuid::new_v4(), "main");
        let mut open = row(&e, site, Some((Uuid::new_v4(), "n", "online")), Some(("s", "running")));
        open.allow_all = true;
        open.alerts = 3;
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &[open], &[], &[], &[]);
        assert_eq!(root.status, "warn");
        let counts = root.counts.clone().unwrap_or_default();
        assert_eq!((counts.allow_all, counts.alerts), (1, 3));
        let degraded = vec![row(&e, site, Some((Uuid::new_v4(), "n", "degraded")), None)];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &degraded, &[], &[], &[]);
        assert_eq!(root.status, "warn");
    }

    #[test]
    fn the_workloads_view_puts_unowned_sandboxes_under_shared() {
        let e = env("Berlin");
        let site = (Uuid::new_v4(), "main");
        let mut owned = row(&e, site, Some((Uuid::new_v4(), "n", "online")), Some(("mine", "running")));
        owned.owner_id = Some(Uuid::new_v4());
        owned.owner_name = Some("ada".into());
        let unowned = row(&e, site, Some((Uuid::new_v4(), "n2", "online")), Some(("free", "running")));
        let root = build_tree(TreeView::Workloads, std::slice::from_ref(&e), &[owned, unowned], &[], &[], &[]);
        let users = &root.children[0].children;
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].name, "ada");
        assert_eq!(users[1].id, "shared");
        assert_eq!(users[1].children[0].name, "free");
    }

    #[test]
    fn a_member_listed_without_sandboxes_appears_in_the_workloads_view() {
        let e = env("Berlin");
        let user = UserRow { environment_id: e.environment_id, user_id: Uuid::new_v4(), display_name: "bob".into(), role: "member".into() };
        let root = build_tree(TreeView::Workloads, std::slice::from_ref(&e), &[], &[user], &[], &[]);
        let bob = &root.children[0].children[0];
        assert_eq!(bob.extra.get("role").and_then(|v| v.as_str()), Some("member"));
    }

    #[test]
    fn the_models_view_nests_installs_under_models_and_clusters() {
        let e = env("Berlin");
        let cluster_id = Uuid::new_v4();
        let install = |node: &str, cluster: Option<Uuid>| InstallRow {
            install_id: Uuid::new_v4(),
            environment_id: e.environment_id,
            model_key: "deepseek".into(),
            model_name: "DeepSeek".into(),
            model_family: "deepseek".into(),
            node_id: Uuid::new_v4(),
            node_name: node.into(),
            state: "installed".into(),
            provider_status: Some("reachable".into()),
            cluster_id: cluster,
        };
        let installs = vec![install("gpu-01", Some(cluster_id)), install("gpu-02", Some(cluster_id)), install("gpu-03", None)];
        let clusters = vec![ClusterRow {
            cluster_id,
            environment_id: e.environment_id,
            name: "x2".into(),
            model_key: "deepseek".into(),
            strategy: "round_robin".into(),
        }];
        let root = build_tree(TreeView::Models, std::slice::from_ref(&e), &[], &[], &installs, &clusters);
        assert_eq!(root.children.len(), 1);
        let model = &root.children[0];
        assert_eq!(model.kind, "model");
        assert_eq!(model.extra.get("installs").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(model.children.len(), 2);
        let cluster = model.children.iter().find(|c| c.kind == "cluster");
        assert_eq!(cluster.map(|c| c.children.len()), Some(2));
        assert!(model.children.iter().any(|c| c.kind == "install"));
    }

    #[test]
    fn a_node_with_too_many_sandboxes_is_truncated() {
        let e = env("Berlin");
        let site = (Uuid::new_v4(), "main");
        let node = (Uuid::new_v4(), "big", "online");
        let rows: Vec<PlacementRow> = (0..MAX_SANDBOXES_PER_NODE + 5)
            .map(|i| row(&e, site, Some(node), Some((&format!("s{i:04}"), "running"))))
            .collect();
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &rows, &[], &[], &[]);
        let node_node = &root.children[0].children[0].children[0];
        assert_eq!(node_node.children.len(), MAX_SANDBOXES_PER_NODE);
        assert_eq!(node_node.extra.get("truncated").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn several_top_organizations_get_a_platform_root_and_a_child_nests_under_its_parent() {
        let parent = env("A");
        let mut child = env("B");
        child.organization_parent = Some(parent.organization_id);
        let other = env("C");
        let envs = vec![parent.clone(), child.clone(), other];
        let root = build_tree(TreeView::Infrastructure, &envs, &[], &[], &[], &[]);
        assert_eq!(root.name, "Platform");
        assert_eq!(root.children.len(), 2);
        let nested = root.children.iter().find(|c| c.id == parent.organization_id.to_string());
        let kinds: Vec<&str> = nested.map(|n| n.children.iter().map(|c| c.kind.as_str()).collect()).unwrap_or_default();
        assert_eq!(kinds, vec!["organization", "data_center"]);
    }

    #[test]
    fn the_sandbox_json_is_flat() {
        let e = env("Berlin");
        let rows = vec![row(&e, (Uuid::new_v4(), "main"), Some((Uuid::new_v4(), "n", "online")), Some(("s", "running")))];
        let root = build_tree(TreeView::Infrastructure, std::slice::from_ref(&e), &rows, &[], &[], &[]);
        let value = serde_json::to_value(&root.children[0].children[0].children[0].children[0]).unwrap_or_default();
        assert_eq!(value["egress_mode"], "forward_proxy");
        assert_eq!(value["kind"], "sandbox");
    }
}
