//! An in-memory runtime adapter for tests: no Docker, no network.
//!
//! It behaves like the real adapter where the services depend on it: `create`
//! records an entry, lifecycle actions change the status, and `logs` returns
//! representative lines. It deliberately does not emulate isolation, because a
//! fake that pretends to be more than it is hides the bugs the real adapter
//! would expose.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use chrono::Utc;
use crate::domain::metrics::{InventoryProbe, ProcessEntry, ProcessTable};
use crate::domain::pack_access::ForwardState;
use crate::domain::runtime::{
    BuildRequest, BuildResult, CatalogueAnswer, CatalogueRequest, ComponentLimits, ComponentState,
    ExecResult, ForwardRequest, HardwareReport, ImageVerification, ModelDownload,
    ModelDownloadResult, ModelServe, ModelServeResult, PackageHit, RuntimeFacts,
    SandboxSpec, SshCredential, VerificationAnswer,
};
use crate::runtime::{BoxTerminal, RuntimeAdapter, RuntimeError, TerminalMessage, TerminalSocket};

/// The runner's report for a set of forwards: every port open, nothing behind it.
fn fake_forward_states(forwards: &[ForwardRequest]) -> Vec<ForwardState> {
    forwards
        .iter()
        .map(|forward| ForwardState {
            port: forward.port,
            target_port: forward.target_port,
            purpose: forward.purpose.clone(),
            listening: true,
            target_up: false,
            error: None,
        })
        .collect()
}

/// Take a lock, recovering from poisoning.
///
/// A poisoned mutex means another test panicked while holding it. In a fake
/// adapter that is not a reason to abort the current test as well, so the guard
/// is recovered and the caller continues.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a fake terminal received and what its other end reported.
///
/// Shared with the adapter by `Arc`, because `open_terminal` takes `&self` and
/// the returned channel outlives the borrow. A test reads the recording to
/// assert that input arrived verbatim, that a resize was applied, and why the
/// session ended.
#[derive(Default)]
pub struct FakeTerminalState {
    feed: Mutex<Option<tokio::sync::mpsc::UnboundedSender<TerminalMessage>>>,
    input: Mutex<Vec<String>>,
    resizes: Mutex<Vec<(u16, u16)>>,
    close: Mutex<Option<String>>,
}

/// In-Memory-Runtime für Unit- und Integrationstests.
#[derive(Default)]
pub struct FakeRuntime {
    sandboxes: Mutex<BTreeMap<String, RuntimeFacts>>,
    specs: Mutex<Vec<SandboxSpec>>,
    /// Model servers that are "running", by key, with the request that started
    /// them, so a test can assert what the service asked for.
    models: Mutex<BTreeMap<String, ModelServe>>,
    downloads: Mutex<Vec<ModelDownload>>,
    /// CUDA-X image pulls as `(component, images)`.
    cudax_installs: Mutex<Vec<(String, Vec<String>)>>,
    /// Backup calls as `(verb, backup_id, root_path)`, so a test can assert what was
    /// asked of the node.
    backups: Mutex<Vec<(String, String, String)>>,
    /// What the control plane asked the runner to hold open, by sandbox, with
    /// the public keys sent alongside.
    forwards: Mutex<BTreeMap<String, (Vec<ForwardRequest>, Vec<String>)>>,
    credentials: Mutex<BTreeMap<String, Vec<SshCredential>>>,
    resource_changes: Mutex<Vec<(String, Option<i64>, Option<i64>, Option<f64>, Option<i64>, Option<i64>)>>,
    terminal: Arc<FakeTerminalState>,
}

impl FakeRuntime {
    /// The backup calls made so far.
    pub fn backup_calls(&self) -> Vec<(String, String, String)> {
        lock(&self.backups).clone()
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Every live resource change asked for, in order.
    pub fn resource_changes(&self) -> Vec<(String, Option<i64>, Option<i64>, Option<f64>, Option<i64>, Option<i64>)> {
        lock(&self.resource_changes).clone()
    }

    /// The temporary logins most recently sent for a sandbox.
    pub fn credentials_for(&self, name: &str) -> Vec<SshCredential> {
        lock(&self.credentials).get(name).cloned().unwrap_or_default()
    }

    /// The forwards and keys most recently sent for a sandbox.
    pub fn forwards_for(&self, name: &str) -> Option<(Vec<ForwardRequest>, Vec<String>)> {
        lock(&self.forwards).get(name).cloned()
    }

    /// Die Specs aller erzeugten Sandboxes – für Tests der Umgebungsvariablen.
    pub fn created_specs(&self) -> Vec<SandboxSpec> {
        lock(&self.specs).clone()
    }

    /// Model servers this fake has been asked to start, by catalog key.
    pub fn served_models(&self) -> BTreeMap<String, ModelServe> {
        lock(&self.models).clone()
    }

    /// Artefacts this fake has been asked to fetch, in order.
    /// What the control plane asked the runner to install, by component.
    pub fn cudax_installs(&self) -> Vec<(String, Vec<String>)> {
        lock(&self.cudax_installs).clone()
    }

    pub fn downloads(&self) -> Vec<ModelDownload> {
        lock(&self.downloads).clone()
    }

    /// Push a frame to the terminal that is open, as the shell would.
    pub fn terminal_say(&self, text: &str) {
        let feed = lock(&self.terminal.feed);
        if let Some(sender) = feed.as_ref() {
            let _ = sender.send(TerminalMessage::Output(text.to_string()));
        }
    }

    /// End the open terminal the way the runner does, with a named reason.
    pub fn terminal_end(&self, reason: &str) {
        let feed = lock(&self.terminal.feed);
        if let Some(sender) = feed.as_ref() {
            let _ = sender.send(TerminalMessage::Closed(reason.to_string()));
        }
    }

    /// The input the terminal received, in order.
    pub fn terminal_input(&self) -> Vec<String> {
        lock(&self.terminal.input).clone()
    }

    /// The resizes the terminal was asked to apply.
    pub fn terminal_resizes(&self) -> Vec<(u16, u16)> {
        lock(&self.terminal.resizes).clone()
    }

    /// Why this end closed the terminal, when it did.
    pub fn terminal_close(&self) -> Option<String> {
        lock(&self.terminal.close).clone()
    }

    fn with_entry<T>(
        &self,
        name: &str,
        change: impl FnOnce(&mut RuntimeFacts) -> T,
    ) -> Result<T, RuntimeError> {
        let mut guard = lock(&self.sandboxes);
        let entry = guard
            .get_mut(name)
            .ok_or_else(|| RuntimeError::NotFound(name.to_string()))?;
        Ok(change(entry))
    }
}

#[async_trait]
impl RuntimeAdapter for FakeRuntime {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn network(&self) -> &str {
        "nestbox-test"
    }

    async fn health(&self) -> Result<(), RuntimeError> {
        Ok(())
    }

    async fn create(&self, spec: SandboxSpec) -> Result<RuntimeFacts, RuntimeError> {
        // Record the spec before its fields move into the facts, so a test can
        // assert what was requested separately from what was reported.
        lock(&self.specs).push(spec.clone());

        let facts = RuntimeFacts {
            id: format!("fake-{}", spec.name),
            name: spec.name.clone(),
            container_name: format!("nestbox-sandbox-{}", spec.name),
            status: "running".to_string(),
            image: spec.image,
            pack_key: spec.pack_key,
            tool_profile: spec.tool_profile,
            ip: "172.30.0.2".to_string(),
            gateway: "172.30.0.1".to_string(),
            uptime_seconds: 1,
            health: "starting".to_string(),
            ..RuntimeFacts::default()
        };
        let mut guard = lock(&self.sandboxes);
        guard.insert(spec.name, facts.clone());
        Ok(facts)
    }

    async fn inspect(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        let guard = lock(&self.sandboxes);
        guard
            .get(name)
            .cloned()
            .ok_or_else(|| RuntimeError::NotFound(name.to_string()))
    }

    async fn list(&self) -> Result<Vec<RuntimeFacts>, RuntimeError> {
        let guard = lock(&self.sandboxes);
        Ok(guard.values().cloned().collect())
    }

    async fn start(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.with_entry(name, |entry| {
            entry.status = "running".to_string();
            entry.clone()
        })
    }

    async fn stop(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.with_entry(name, |entry| {
            entry.status = "exited".to_string();
            entry.uptime_seconds = 0;
            entry.clone()
        })
    }

    async fn restart(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.with_entry(name, |entry| {
            entry.status = "running".to_string();
            entry.clone()
        })
    }

    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let mut guard = lock(&self.sandboxes);
        guard
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| RuntimeError::NotFound(name.to_string()))
    }

    async fn logs(&self, name: &str, tail: u32) -> Result<String, RuntimeError> {
        let guard = lock(&self.sandboxes);
        if !guard.contains_key(name) {
            return Err(RuntimeError::NotFound(name.to_string()));
        }
        Ok(format!("{name}: last {tail} lines (fake output)"))
    }

    async fn exec(
        &self,
        name: &str,
        command: &[String],
        _timeout_seconds: u32,
    ) -> Result<ExecResult, RuntimeError> {
        let guard = lock(&self.sandboxes);
        if !guard.contains_key(name) {
            return Err(RuntimeError::NotFound(name.to_string()));
        }
        // Echo the vector back joined by spaces so a test can assert that the
        // argument boundaries survived the trip.
        Ok(ExecResult {
            exit_code: Some(0),
            output: command.join(" "),
            truncated: false,
        })
    }

    async fn hardware(&self) -> Result<HardwareReport, RuntimeError> {
        // A small x86 host with no GPU: the shape the appliance VM has.
        Ok(HardwareReport {
            hostname: Some("fake-node".into()),
            cpuinfo: "processor\t: 0\nmodel name\t: Fake CPU\nphysical id\t: 0\ncore id\t\t: 0\n\n\
                      processor\t: 1\nmodel name\t: Fake CPU\nphysical id\t: 0\ncore id\t\t: 1\n"
                .into(),
            meminfo: "MemTotal:        8388608 kB\n".into(),
            nvidia_smi_csv: None,
            nvidia_smi_header: None,
            container_gpu: false,
            note: Some("fake runtime: no GPU".into()),
        })
    }

    async fn build(&self, request: BuildRequest) -> Result<BuildResult, RuntimeError> {
        Ok(BuildResult {
            tag: request.tag,
            image_id: Some("sha256:fake".into()),
            log: "fake build".into(),
            success: true,
        })
    }

    async fn catalogue(&self, request: CatalogueRequest) -> Result<CatalogueAnswer, RuntimeError> {
        // The fake answers with a tiny catalogue so the console's search path is
        // testable without a package index or a container runtime.
        let needle = request.query.to_lowercase();
        let items = ["git", "curl", "jq", "vim", "python3"]
            .iter()
            .filter(|name| name.contains(&needle))
            .map(|name| PackageHit {
                name: (*name).to_string(),
                version: Some("1".into()),
                summary: Some("fake package".into()),
            })
            .collect();
        Ok(CatalogueAnswer {
            items,
            available: true,
            detail: format!("fake {} catalogue", request.package_manager),
        })
    }

    async fn verify_image(
        &self,
        request: ImageVerification,
    ) -> Result<VerificationAnswer, RuntimeError> {
        Ok(VerificationAnswer {
            present: request.packages,
            missing: Vec::new(),
            note: Some("fake verification".into()),
        })
    }

    async fn processes(&self, name: &str) -> Result<ProcessTable, RuntimeError> {
        // A stopped sandbox has no table; the fake answers like the runner does
        // rather than inventing rows for a container that is not there.
        let guard = lock(&self.sandboxes);
        let facts = guard
            .get(name)
            .ok_or_else(|| RuntimeError::NotFound(name.to_string()))?;
        if facts.status != "running" {
            return Err(RuntimeError::Rejected(format!(
                "the sandbox is '{}'",
                facts.status
            )));
        }

        Ok(ProcessTable {
            taken_at: Utc::now(),
            columns: vec![
                "PID".into(),
                "PPID".into(),
                "USER".into(),
                "%CPU".into(),
                "%MEM".into(),
                "ELAPSED".into(),
                "COMMAND".into(),
            ],
            items: vec![
                ProcessEntry {
                    pid: 1,
                    ppid: None,
                    user: "nestbox".into(),
                    cpu_pct: Some(0.1),
                    mem_pct: Some(0.4),
                    elapsed: Some("00:04:12".into()),
                    command: "hermes serve --port 8080".into(),
                },
                ProcessEntry {
                    pid: 42,
                    ppid: Some(1),
                    user: "nestbox".into(),
                    cpu_pct: Some(0.0),
                    mem_pct: Some(0.1),
                    elapsed: Some("00:04:05".into()),
                    command: "node /usr/local/bin/mcp-hub".into(),
                },
            ],
            truncated: false,
        })
    }

    async fn process_action(&self, name: &str, pid: i64, action: &str, nice: Option<i8>) -> Result<(), RuntimeError> {
        self.processes(name).await?;
        if pid < 2 { return Err(RuntimeError::Rejected("PID 1 is protected".into())); }
        if !["terminate", "kill", "pause", "resume", "set_nice"].contains(&action) { return Err(RuntimeError::Rejected("unknown action".into())); }
        if action == "set_nice" && nice.is_none() { return Err(RuntimeError::Rejected("nice required".into())); }
        Ok(())
    }

    async fn inventory(&self, name: &str) -> Result<InventoryProbe, RuntimeError> {
        let guard = lock(&self.sandboxes);
        if !guard.contains_key(name) {
            return Err(RuntimeError::NotFound(name.to_string()));
        }

        Ok(InventoryProbe {
            taken_at: Utc::now(),
            facts: serde_json::json!({
                "os": {
                    "ID": "ubuntu",
                    "NAME": "Ubuntu",
                    "VERSION_ID": "24.04",
                    "PRETTY_NAME": "Ubuntu 24.04.1 LTS",
                    "kernel": "Linux 6.8.0-45-generic",
                    "arch": "x86_64",
                },
                "packages": [
                    { "name": "bash", "version": "5.2.21-2ubuntu4" },
                    { "name": "git", "version": "1:2.43.0-1ubuntu7.2" },
                    { "name": "python3", "version": "3.12.3-0ubuntu2" },
                ],
                "agents": {
                    "hermes": "hermes 0.22.5",
                    "node": "v22.11.0",
                    "python3": "Python 3.12.3",
                    "git": "git version 2.43.0",
                },
                "listening": [
                    { "address": "0.0.0.0:8080", "protocol": "tcp" },
                    { "address": "[::]:22", "protocol": "tcp" },
                ],
                "env_keys": ["HOME", "PATH", "HERMES_PORT"],
                "mounts": [
                    { "target": "/home/nestbox", "fstype": "ext4", "source": "/dev/sda1" }
                ],
                "image": {
                    "reference": "nestbox/tool-hermes:0.22.5",
                    "id": "sha256:fake",
                    "created": "2026-10-20T09:00:00Z",
                    "labels": { "nestbox.pack": "hermes" },
                },
            }),
            note: None,
        })
    }

    async fn download_model(
        &self,
        request: ModelDownload,
    ) -> Result<ModelDownloadResult, RuntimeError> {
        // The fake claims the bytes without fetching them: a test that wants to
        // know what was asked reads `downloads()`, not the filesystem.
        lock(&self.downloads).push(request.clone());
        Ok(ModelDownloadResult {
            path: format!("/models/{}/{}", request.model_key, request.file),
            bytes: 1024,
            cached: false,
        })
    }

    async fn model_logs(&self, model_key: &str, tail: u32) -> Result<String, RuntimeError> {
        if !lock(&self.models).contains_key(model_key) {
            return Err(RuntimeError::NotFound(format!("model-{model_key}")));
        }
        Ok(format!("model-{model_key}: last {tail} lines (fake output)"))
    }

    async fn model_metrics(
        &self,
        model_key: &str,
    ) -> Result<serde_json::Value, RuntimeError> {
        if !lock(&self.models).contains_key(model_key) {
            return Err(RuntimeError::Rejected(format!(
                "no model server for {model_key}"
            )));
        }
        Ok(serde_json::json!({
            "model_key": model_key,
            "metrics": {
                "llamacpp:prompt_tokens_total": 120.0,
                "llamacpp:tokens_predicted_total": 48.0,
                "llamacpp:requests_processing": 1.0,
                "llamacpp:kv_cache_usage_ratio": 0.25
            }
        }))
    }

    async fn install_libraries(
        &self,
        component: &str,
        images: &[String],
    ) -> Result<Vec<String>, RuntimeError> {
        lock(&self.cudax_installs).push((component.to_string(), images.to_vec()));
        Ok(images.to_vec())
    }

    async fn serve_model(&self, request: ModelServe) -> Result<ModelServeResult, RuntimeError> {
        let name = format!("model-{}", request.model_key.replace(['.', ':'], "-"));
        lock(&self.models).insert(request.model_key.clone(), request);
        Ok(ModelServeResult {
            name,
            container_id: Some("fake-container".into()),
            status: "running".into(),
            published: None,
        })
    }

    async fn stop_model(&self, model_key: &str) -> Result<(), RuntimeError> {
        lock(&self.models).remove(model_key);
        Ok(())
    }

    async fn set_forwards(
        &self,
        name: &str,
        forwards: &[ForwardRequest],
        authorized_keys: &[String],
        credentials: &[SshCredential],
    ) -> Result<Vec<ForwardState>, RuntimeError> {
        lock(&self.credentials).insert(name.to_string(), credentials.to_vec());
        lock(&self.forwards).insert(name.to_string(), (forwards.to_vec(), authorized_keys.to_vec()));
        Ok(fake_forward_states(forwards))
    }

    async fn forward_states(&self, name: &str) -> Result<Vec<ForwardState>, RuntimeError> {
        let held = lock(&self.forwards).get(name).map(|(forwards, _)| forwards.clone());
        Ok(held.map(|forwards| fake_forward_states(&forwards)).unwrap_or_default())
    }

    async fn clear_forwards(&self, name: &str) -> Result<(), RuntimeError> {
        lock(&self.forwards).remove(name);
        Ok(())
    }

    async fn backup_create(&self, root_path: &str, backup_id: &str) -> Result<(i64, String), RuntimeError> {
        lock(&self.backups).push(("create".into(), backup_id.into(), root_path.into()));
        Ok((4096, "ab".repeat(32)))
    }

    async fn backup_restore(
        &self,
        backup_id: &str,
        root_path: &str,
        _sha256: &str,
        replace: bool,
    ) -> Result<serde_json::Value, RuntimeError> {
        lock(&self.backups).push((if replace { "restore-replace" } else { "restore" }.into(), backup_id.into(), root_path.into()));
        Ok(serde_json::json!({ "restored": 3, "previous_content_kept_at": null }))
    }

    async fn backup_delete(&self, backup_id: &str) -> Result<(), RuntimeError> {
        lock(&self.backups).push(("delete".into(), backup_id.into(), String::new()));
        Ok(())
    }

    async fn host_load(&self) -> Result<serde_json::Value, RuntimeError> {
        Ok(serde_json::json!({
            "cpu_percent": 12.5, "cpus": 4,
            "memory": { "total": 8_000_000_000_i64, "available": 4_000_000_000_i64, "used": 4_000_000_000_i64, "swap_total": 0 },
            "disk": { "total": 116_000_000_000_i64, "used": 16_000_000_000_i64, "free": 100_000_000_000_i64 },
            "sandboxes": { "total": 1, "running": 1 },
            "promised": { "memory_bytes": 1_000_000_000_i64, "cpus": 1.0 }
        }))
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
        lock(&self.resource_changes).push((
            name.to_string(),
            memory_bytes,
            memory_reservation_bytes,
            cpus,
            cpu_shares,
            pids_limit,
        ));
        Ok(())
    }

    async fn components(&self) -> Result<Vec<ComponentState>, RuntimeError> {
        let row = |service: &str, description: &str| ComponentState {
            service: service.to_string(),
            description: description.to_string(),
            container: Some(format!("nestbox-{service}-1")),
            image: Some(format!("nestbox-{service}")),
            state: "running".to_string(),
            health: None,
            started_at: None,
            restarts: 0,
            ports: serde_json::json!([]),
            cpu_pct: Some(1.5),
            mem_bytes: Some(64 * 1024 * 1024),
            mem_limit_bytes: Some(512 * 1024 * 1024),
            pids: Some(4),
            limits: serde_json::json!({"memory_limit_bytes": 512 * 1024 * 1024}),
        };
        Ok(vec![
            row("control-api", "The control plane."),
            row("gateway", "Front door."),
            row("sandbox-runner", "Runs sandboxes."),
            row("egress-proxy", "The only way out."),
        ])
    }

    async fn set_component_limits(
        &self,
        service: &str,
        limits: ComponentLimits,
    ) -> Result<(), RuntimeError> {
        if limits.is_empty() {
            return Err(RuntimeError::Rejected("nothing to change".into()));
        }
        let known = self
            .components()
            .await?
            .into_iter()
            .any(|component| component.service == service);
        if !known {
            return Err(RuntimeError::NotFound(service.to_string()));
        }
        Ok(())
    }

    async fn restart_component(&self, service: &str) -> Result<(), RuntimeError> {
        let known = self
            .components()
            .await?
            .into_iter()
            .any(|component| component.service == service);
        if !known {
            return Err(RuntimeError::NotFound(service.to_string()));
        }
        Ok(())
    }

    async fn component_logs(&self, service: &str, _tail: u32) -> Result<String, RuntimeError> {
        Ok(format!("{service}: ready\n"))
    }

    async fn open_terminal(&self, name: &str, _tab_id: &str, _who: &str) -> Result<BoxTerminal, RuntimeError> {
        let guard = lock(&self.sandboxes);
        if !guard.contains_key(name) {
            return Err(RuntimeError::NotFound(name.to_string()));
        }
        drop(guard);

        // One session at a time is enough to test the rules: a second open would
        // simply replace the feed, which is what a real runner would not do but
        // a test never needs.
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        *lock(&self.terminal.feed) = Some(sender);
        *lock(&self.terminal.close) = None;
        lock(&self.terminal.input).clear();
        lock(&self.terminal.resizes).clear();
        Ok(Box::new(FakeTerminal {
            receiver,
            state: Arc::clone(&self.terminal),
        }))
    }
}

/// The adapter side of a fake terminal: the test holds the other end.
struct FakeTerminal {
    receiver: tokio::sync::mpsc::UnboundedReceiver<TerminalMessage>,
    state: Arc<FakeTerminalState>,
}

#[async_trait]
impl TerminalSocket for FakeTerminal {
    async fn recv(&mut self) -> Result<Option<TerminalMessage>, RuntimeError> {
        Ok(self.receiver.recv().await)
    }

    async fn send_input(&mut self, text: &str) -> Result<(), RuntimeError> {
        lock(&self.state.input).push(text.to_string());
        Ok(())
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), RuntimeError> {
        lock(&self.state.resizes).push((cols, rows));
        Ok(())
    }

    async fn close(&mut self, reason: &str) {
        *lock(&self.state.close) = Some(reason.to_string());
    }
}
