//! The dispatcher: one `RuntimeAdapter` in front of one runner per node (ADR 0047).
//!
//! Services keep calling `ctx.runtime` and keep naming sandboxes by name. The router answers
//! the question "which machine?" and forwards the call there, so nothing above this file
//! learned that there is more than one machine.
//!
//! * A call about one sandbox goes to the runner of the node the sandbox is placed on. The
//!   placement is read from the database once (`nb_node_of_sandbox`) and remembered.
//! * `create` carries its node in the spec, because the row is not committed yet.
//! * A call about nothing in particular (a build, a catalogue, host load) goes to the local
//!   runner, as it always did. A caller that means a certain node asks for it by id with
//!   `for_node`.
//! * `list` merges every runner that answers, so the reconcile loop sees every sandbox. A
//!   node that does not answer contributes nothing and the others still do; an unreachable
//!   node must not make its neighbours' sandboxes look gone.
//!
//! It does not provide: placement decisions (the caller names the node), failover of a sandbox
//! to another node, or any state of its own beyond the two caches.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use uuid::Uuid;

use super::http_runner::HttpRunner;
use super::{BoxTerminal, RuntimeAdapter, RuntimeError, VncStream};
use crate::domain::metrics::{InventoryProbe, ProcessTable};
use crate::domain::pack_access::ForwardState;
use crate::domain::runtime::{
    BuildRequest, BuildResult, CatalogueAnswer, CatalogueRequest, ComponentLimits, ComponentState,
    ExecResult, ForwardRequest, HardwareReport, ImageVerification, ModelDownload,
    ModelDownloadResult, ModelServe, ModelServeResult, RuntimeFacts, SandboxSpec, SshCredential,
    VerificationAnswer,
};

/// A remote node as the router needs it.
#[derive(Debug, Clone)]
pub struct RemoteNode {
    pub id: Uuid,
    pub runner_url: String,
    pub token: String,
}

/// See the module comment.
pub struct RuntimeRouter {
    local: Arc<dyn RuntimeAdapter>,
    network: String,
    remotes: RwLock<HashMap<Uuid, (RemoteNode, Arc<dyn RuntimeAdapter>)>>,
    placement: RwLock<HashMap<String, Option<Uuid>>>,
    pool: Option<sqlx::PgPool>,
}

impl RuntimeRouter {
    /// A router whose local runner is `local`. Without a pool every sandbox is local, which is
    /// what the unit tests and a single-machine install without joined nodes need.
    pub fn new(local: Arc<dyn RuntimeAdapter>, pool: Option<sqlx::PgPool>) -> Self {
        let network = local.network().to_string();
        Self { local, network, remotes: RwLock::new(HashMap::new()), placement: RwLock::new(HashMap::new()), pool }
    }

    /// Replace the set of remote nodes. An unchanged node keeps its adapter.
    pub fn set_remotes(&self, nodes: Vec<RemoteNode>) {
        let Ok(mut remotes) = self.remotes.write() else { return };
        let mut next = HashMap::new();
        for node in nodes {
            let adapter = match remotes.remove(&node.id) {
                Some((old, adapter)) if old.runner_url == node.runner_url && old.token == node.token => adapter,
                _ => Arc::new(HttpRunner::new(node.runner_url.clone(), self.network.clone()).with_token(node.token.clone()))
                    as Arc<dyn RuntimeAdapter>,
            };
            next.insert(node.id, (node, adapter));
        }
        *remotes = next;
    }

    /// The nodes the router can reach, for the health loop.
    pub fn remote_ids(&self) -> Vec<Uuid> {
        self.remotes.read().map(|r| r.keys().copied().collect()).unwrap_or_default()
    }

    /// The runner of one node: the local one for `None` or an unknown id that is not remote.
    pub fn for_node(&self, node: Option<Uuid>) -> Arc<dyn RuntimeAdapter> {
        if let Some(id) = node {
            if let Ok(remotes) = self.remotes.read() {
                if let Some((_, adapter)) = remotes.get(&id) {
                    return adapter.clone();
                }
            }
        }
        self.local.clone()
    }

    /// The host load of one node's runner. Wave 0 stub for package B07: the local node answers,
    /// a remote runner is reported unreachable until B07 routes the call to it.
    pub async fn host_load_for(&self, node: Uuid) -> Result<serde_json::Value, RuntimeError> {
        if self.is_remote(node) {
            return Err(RuntimeError::Unavailable("not built: B07".into()));
        }
        self.local.host_load().await
    }

    /// Whether a node id is one of the remote runners.
    pub fn is_remote(&self, node: Uuid) -> bool {
        self.remotes.read().map(|r| r.contains_key(&node)).unwrap_or(false)
    }

    /// Remember where a sandbox runs, so the next call needs no query.
    pub fn place(&self, name: &str, node: Option<Uuid>) {
        if let Ok(mut placement) = self.placement.write() {
            placement.insert(name.to_string(), node);
        }
    }

    fn forget(&self, name: &str) {
        if let Ok(mut placement) = self.placement.write() {
            placement.remove(name);
        }
    }

    async fn adapter_for(&self, name: &str) -> Arc<dyn RuntimeAdapter> {
        let cached = self.placement.read().ok().and_then(|p| p.get(name).copied());
        let node = match cached {
            Some(node) => node,
            None => {
                let found = match &self.pool {
                    Some(pool) => sqlx::query_scalar::<_, Option<Uuid>>("SELECT nb_node_of_sandbox($1)")
                        .bind(name)
                        .fetch_one(pool)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                };
                // Only a remote answer is worth remembering: a local sandbox is the default, and
                // remembering "local" for a name that is simply not committed yet would stick.
                if found.is_some_and(|id| self.is_remote(id)) {
                    self.place(name, found);
                }
                found
            }
        };
        self.for_node(node)
    }

    fn all(&self) -> Vec<Arc<dyn RuntimeAdapter>> {
        let mut out = vec![self.local.clone()];
        if let Ok(remotes) = self.remotes.read() {
            out.extend(remotes.values().map(|(_, a)| a.clone()));
        }
        out
    }
}

#[async_trait]
impl RuntimeAdapter for RuntimeRouter {
    fn name(&self) -> &'static str {
        self.local.name()
    }

    fn network(&self) -> &str {
        &self.network
    }

    async fn health(&self) -> Result<(), RuntimeError> {
        self.local.health().await
    }

    async fn create(&self, spec: SandboxSpec) -> Result<RuntimeFacts, RuntimeError> {
        let adapter = self.for_node(spec.node_id);
        if spec.node_id.is_some_and(|id| self.is_remote(id)) {
            self.place(&spec.name, spec.node_id);
        }
        adapter.create(spec).await
    }

    async fn inspect(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.adapter_for(name).await.inspect(name).await
    }

    async fn list(&self) -> Result<Vec<RuntimeFacts>, RuntimeError> {
        let mut merged = self.local.list().await?;
        let remotes: Vec<_> = self.all().into_iter().skip(1).collect();
        for adapter in remotes {
            if let Ok(mut more) = tokio::time::timeout(std::time::Duration::from_secs(5), adapter.list()).await.unwrap_or(Err(RuntimeError::Unavailable("timeout".into()))) {
                merged.append(&mut more);
            }
        }
        Ok(merged)
    }

    async fn start(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.adapter_for(name).await.start(name).await
    }
    async fn stop(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.adapter_for(name).await.stop(name).await
    }
    async fn restart(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.adapter_for(name).await.restart(name).await
    }
    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let result = self.adapter_for(name).await.remove(name).await;
        self.forget(name);
        result
    }
    async fn logs(&self, name: &str, tail: u32) -> Result<String, RuntimeError> {
        self.adapter_for(name).await.logs(name, tail).await
    }
    async fn exec(&self, name: &str, command: &[String], timeout_seconds: u32) -> Result<ExecResult, RuntimeError> {
        self.adapter_for(name).await.exec(name, command, timeout_seconds).await
    }

    async fn model_logs(&self, model_key: &str, tail: u32) -> Result<String, RuntimeError> {
        self.local.model_logs(model_key, tail).await
    }
    async fn model_metrics(&self, model_key: &str) -> Result<serde_json::Value, RuntimeError> {
        self.local.model_metrics(model_key).await
    }
    async fn install_libraries(&self, component: &str, images: &[String]) -> Result<Vec<String>, RuntimeError> {
        self.local.install_libraries(component, images).await
    }
    async fn hardware(&self) -> Result<HardwareReport, RuntimeError> {
        self.local.hardware().await
    }
    async fn build(&self, request: BuildRequest) -> Result<BuildResult, RuntimeError> {
        self.local.build(request).await
    }
    async fn catalogue(&self, request: CatalogueRequest) -> Result<CatalogueAnswer, RuntimeError> {
        self.local.catalogue(request).await
    }
    async fn verify_image(&self, request: ImageVerification) -> Result<VerificationAnswer, RuntimeError> {
        self.local.verify_image(request).await
    }
    async fn processes(&self, name: &str) -> Result<ProcessTable, RuntimeError> {
        self.adapter_for(name).await.processes(name).await
    }
    async fn open_vnc(&self, name: &str, view_only: bool) -> Result<VncStream, RuntimeError> {
        self.adapter_for(name).await.open_vnc(name, view_only).await
    }
    async fn remote_ssh_run(
        &self,
        request: crate::domain::runtime::RemoteSshRun,
    ) -> Result<crate::domain::runtime::ExecResult, RuntimeError> {
        // Only the appliance's own runner reaches other machines: the node that
        // is being installed does not exist yet, so there is no adapter for it.
        self.local.remote_ssh_run(request).await
    }
    async fn live_view(&self, name: &str, kind: &str) -> Result<serde_json::Value, RuntimeError> {
        self.adapter_for(name).await.live_view(name, kind).await
    }
    async fn process_action(&self, name: &str, pid: i64, action: &str, nice: Option<i8>) -> Result<(), RuntimeError> {
        self.adapter_for(name).await.process_action(name, pid, action, nice).await
    }
    async fn inventory(&self, name: &str) -> Result<InventoryProbe, RuntimeError> {
        self.adapter_for(name).await.inventory(name).await
    }
    async fn download_model(&self, request: ModelDownload) -> Result<ModelDownloadResult, RuntimeError> {
        // The artefact has to land on the machine that will serve it: a joined
        // node has its own disk and its own model base, so fetching here would
        // leave the node with no file to start. The install names the node.
        self.for_node(request.node_id).download_model(request).await
    }
    async fn serve_model(&self, request: ModelServe) -> Result<ModelServeResult, RuntimeError> {
        // The same node as the download, because that is where the file is.
        self.for_node(request.node_id).serve_model(request).await
    }
    async fn stop_model(&self, model_key: &str) -> Result<(), RuntimeError> {
        // A model is named by its key alone, and two nodes may serve the same
        // key, so there is nothing to choose with. Stopping stays local; a
        // server on a joined node keeps running until that runner is asked
        // directly (see the B17 report).
        self.local.stop_model(model_key).await
    }
    async fn open_terminal(&self, name: &str, tab_id: &str, who: &str) -> Result<BoxTerminal, RuntimeError> {
        self.adapter_for(name).await.open_terminal(name, tab_id, who).await
    }

    async fn terminal_sessions(&self, name: &str) -> Result<serde_json::Value, RuntimeError> {
        self.adapter_for(name).await.terminal_sessions(name).await
    }
    async fn set_forwards(
        &self,
        name: &str,
        forwards: &[ForwardRequest],
        authorized_keys: &[String],
        credentials: &[SshCredential],
    ) -> Result<Vec<ForwardState>, RuntimeError> {
        self.adapter_for(name).await.set_forwards(name, forwards, authorized_keys, credentials).await
    }
    async fn forward_states(&self, name: &str) -> Result<Vec<ForwardState>, RuntimeError> {
        self.adapter_for(name).await.forward_states(name).await
    }
    async fn host_load(&self) -> Result<serde_json::Value, RuntimeError> {
        self.local.host_load().await
    }
    async fn update_resources(
        &self,
        name: &str,
        memory_bytes: Option<i64>,
        memory_reservation_bytes: Option<i64>,
        cpus: Option<f64>,
        cpu_shares: Option<i64>,
        pids_limit: Option<i64>,
    ) -> Result<(), RuntimeError> {
        self.adapter_for(name)
            .await
            .update_resources(name, memory_bytes, memory_reservation_bytes, cpus, cpu_shares, pids_limit)
            .await
    }
    async fn backup_create(&self, root_path: &str, backup_id: &str) -> Result<(i64, String), RuntimeError> {
        self.local.backup_create(root_path, backup_id).await
    }
    async fn backup_restore(&self, backup_id: &str, root_path: &str, sha256: &str, replace: bool) -> Result<serde_json::Value, RuntimeError> {
        self.local.backup_restore(backup_id, root_path, sha256, replace).await
    }
    async fn backup_delete(&self, backup_id: &str) -> Result<(), RuntimeError> {
        self.local.backup_delete(backup_id).await
    }
    async fn openshell_view(&self, name: &str) -> Result<serde_json::Value, RuntimeError> {
        self.adapter_for(name).await.openshell_view(name).await
    }
    async fn openshell_events(&self, since_seconds: u32) -> Result<Vec<serde_json::Value>, RuntimeError> {
        self.local.openshell_events(since_seconds).await
    }
    async fn set_openshell_policy(&self, name: &str, document: &serde_json::Value) -> Result<(), RuntimeError> {
        self.adapter_for(name).await.set_openshell_policy(name, document).await
    }
    async fn clear_forwards(&self, name: &str) -> Result<(), RuntimeError> {
        self.adapter_for(name).await.clear_forwards(name).await
    }
    async fn components(&self) -> Result<Vec<ComponentState>, RuntimeError> {
        self.local.components().await
    }
    async fn images(&self) -> Result<Vec<crate::domain::runtime::NodeImageFact>, RuntimeError> {
        self.local.images().await
    }
    async fn prune_snapshots(&self, name: &str, password: &str, keep_last: i32, keep_daily: i32) -> Result<serde_json::Value, RuntimeError> {
        self.local.prune_snapshots(name, password, keep_last, keep_daily).await
    }
    async fn snapshot_backup(&self, repo: &str, password: &str, path: &str, tags: &[String]) -> Result<crate::domain::runtime::SnapshotFact, RuntimeError> {
        self.local.snapshot_backup(repo, password, path, tags).await
    }
    async fn snapshot_restore(&self, repo: &str, password: &str, snapshot: &str, source_path: &str, path: &str) -> Result<serde_json::Value, RuntimeError> {
        self.local.snapshot_restore(repo, password, snapshot, source_path, path).await
    }
    async fn pull_image(&self, image: &str) -> Result<crate::domain::runtime::NodeImageFact, RuntimeError> {
        self.local.pull_image(image).await
    }
    async fn component_logs(&self, service: &str, tail: u32) -> Result<String, RuntimeError> {
        self.local.component_logs(service, tail).await
    }
    async fn set_component_limits(&self, service: &str, limits: ComponentLimits) -> Result<(), RuntimeError> {
        self.local.set_component_limits(service, limits).await
    }
    async fn restart_component(&self, service: &str) -> Result<(), RuntimeError> {
        self.local.restart_component(service).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::fake::FakeRuntime;

    fn spec(name: &str, node: Option<Uuid>) -> SandboxSpec {
        SandboxSpec {
            name: name.into(),
            image: "ubuntu:24.04".into(),
            command: Vec::new(),
            environment: Vec::new(),
            pack_key: "ubuntu-dev".into(),
            tool_profile: String::new(),
            memory_limit: "512Mi".into(),
            cpu_limit: 0.5,
            pids_limit: 64,
            mounts: Vec::new(),
            gpu_count: 0,
            egress_mode: "forward_proxy".into(),
            node_id: node,
            memory_reservation: None,
            cpu_shares: None,
        }
    }

    #[tokio::test]
    async fn without_remote_nodes_everything_is_local() {
        let local = Arc::new(FakeRuntime::new());
        let router = RuntimeRouter::new(local.clone(), None);
        router.create(spec("a", None)).await.expect("create");
        assert!(local.inspect("a").await.is_ok());
        assert_eq!(router.list().await.expect("list").len(), 1);
        // A node id the router does not know is the local node, not an error.
        router.create(spec("b", Some(Uuid::new_v4()))).await.expect("create");
        assert!(local.inspect("b").await.is_ok());
    }

    #[tokio::test]
    async fn a_remote_node_that_does_not_answer_does_not_hide_the_others() {
        let local = Arc::new(FakeRuntime::new());
        let router = RuntimeRouter::new(local.clone(), None);
        let node = Uuid::new_v4();
        // Nothing listens there: every call to it fails fast.
        router.set_remotes(vec![RemoteNode { id: node, runner_url: "http://127.0.0.1:9".into(), token: "t-0123456789".into() }]);
        assert!(router.is_remote(node));
        router.create(spec("local-one", None)).await.expect("local create");
        assert!(router.create(spec("far-one", Some(node))).await.is_err(), "the remote create fails, it does not fall back to local");
        assert!(local.inspect("far-one").await.is_err(), "and nothing was created locally");
        let listed = router.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "local-one");
    }

    #[tokio::test]
    async fn a_removed_sandbox_is_forgotten() {
        let local = Arc::new(FakeRuntime::new());
        let router = RuntimeRouter::new(local, None);
        let node = Uuid::new_v4();
        router.place("x", Some(node));
        let _ = router.remove("x").await;
        assert!(router.placement.read().expect("lock").get("x").is_none());
    }
}
