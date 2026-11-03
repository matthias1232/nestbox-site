//! Docker-Adapter über den Sandbox-Runner.
//!
//! Der Runner ist ein kleiner Dienst mit Docker-Socket und führt die
//! Container-Operationen aus. Diese Datei übersetzt zwischen dem
//! [`RuntimeAdapter`]-Vertrag und dessen HTTP-Endpunkten.

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use chrono::{DateTime, Utc};
use crate::domain::metrics::{InventoryProbe, ProcessTable};
use crate::domain::pack_access::ForwardState;
use crate::domain::runtime::{
    BuildRequest, BuildResult, CatalogueAnswer, CatalogueRequest, ComponentLimits,
    ComponentState, ExecResult, ForwardRequest, HardwareReport, ImageVerification, ModelDownload,
    ModelDownloadResult, ModelServe, ModelServeResult, NetworkInterface, RuntimeFacts,
    RuntimeResources, SandboxSpec, SshCredential, VerificationAnswer,
};
use crate::runtime::{BoxTerminal, RunnerTerminal, RuntimeAdapter, RuntimeError};

/// Antwort des Runners für eine Sandbox.
#[derive(Debug, Deserialize)]
struct RunnerSandbox {
    id: String,
    name: String,
    #[serde(default)]
    container_name: String,
    status: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    pack_key: String,
    #[serde(default)]
    tool_profile: String,
    #[serde(default)]
    uptime_seconds: u64,
    #[serde(default)]
    health: String,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    exit_code: Option<i64>,
    #[serde(default)]
    network: Vec<NetworkInterface>,
    #[serde(default)]
    resources: RuntimeResources,
    #[serde(default)]
    supervisor_status: String,
    #[serde(default)]
    supervisor_image: String,
}

impl From<RunnerSandbox> for RuntimeFacts {
    fn from(value: RunnerSandbox) -> Self {
        let first = value.network.first().cloned().unwrap_or_default();
        Self {
            id: value.id,
            name: value.name,
            container_name: value.container_name,
            status: value.status,
            image: value.image,
            pack_key: value.pack_key,
            tool_profile: value.tool_profile,
            ip: first.ip.clone(),
            gateway: first.gateway.clone(),
            network: value.network,
            uptime_seconds: value.uptime_seconds,
            health: value.health,
            started_at: value.started_at,
            exit_code: value.exit_code,
            resources: value.resources,
            supervisor_status: value.supervisor_status,
            supervisor_image: value.supervisor_image,
        }
    }
}

/// The bound every runner call shares.
///
/// A hung runner must not hold an API worker forever. Docker's own stop grace
/// period is 10 s and an exec may run for minutes, so the bound is generous but
/// finite.
const CLIENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(660);

/// The bound of one recipe build.
///
/// A build is not an ordinary call: it runs `apt-get`, Chrome and the desktop
/// pack inside Docker and took about eleven minutes on the appliance, which is
/// longer than [`CLIENT_TIMEOUT`]. The API answers the operator the moment the
/// job starts, so a long wait here holds no request open, only the job.
const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3600);

/// Adapter, der einen HTTP-fähigen Runtime-Dienst anspricht.
pub struct HttpRunner {
    base_url: String,
    network: String,
    client: reqwest::Client,
    /// The remote node's own token. The local runner is reached on a private compose network
    /// and uses the node token file instead; a runner on another machine is reached over the
    /// overlay and demands its token on every request (ADR 0047).
    token: Option<String>,
}

impl HttpRunner {
    pub fn new(base_url: impl Into<String>, network: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            network: network.into(),
            // A hung runner must not hold an API worker forever. Docker's own
            // stop grace period is 10 s and an exec may run for minutes, so the
            // bound is generous but finite. A build gets its own, longer one
            // (see `send_with_timeout` and [`BUILD_TIMEOUT`]).
            client: reqwest::Client::builder()
                .timeout(CLIENT_TIMEOUT)
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            token: None,
        }
    }

    /// The same runner, authenticated with the token of the node it runs on.
    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        self.token = Some(token).filter(|t| !t.trim().is_empty());
        self
    }

    /// The token to present to this runner: its own if it has one, else the local node token.
    fn node_token(&self) -> Option<String> {
        self.token.clone().or_else(read_node_token)
    }

    /// The same host and port, with the WebSocket scheme.
    fn ws_base_url(&self) -> String {
        if let Some(rest) = self.base_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            format!("ws://{}", self.base_url)
        }
    }

    async fn send<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T, RuntimeError> {
        self.send_with_timeout(method, path, body, CLIENT_TIMEOUT).await
    }

    /// The same call with its own bound instead of the client-wide one.
    ///
    /// Only a recipe build needs this: it installs apt packages, Chrome and the
    /// desktop pack inside Docker, which took about eleven minutes on the
    /// appliance and would be cut off by the 660 s bound every other call
    /// shares. The API has already answered the operator by then, so waiting
    /// here holds no request open.
    async fn send_with_timeout<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
        timeout: std::time::Duration,
    ) -> Result<T, RuntimeError> {
        let url = format!("{}{}", self.base_url, path);
        let mut request = self.client.request(method, url).timeout(timeout);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(payload) = body {
            request = request.json(payload);
        }
        let response = request
            .send()
            .await
            .map_err(|error| RuntimeError::Unavailable(error.to_string()))?;

        let status = response.status();
        let raw = response
            .text()
            .await
            .map_err(|error| RuntimeError::Unavailable(error.to_string()))?;

        if status == StatusCode::NOT_FOUND {
            return Err(RuntimeError::NotFound(raw));
        }
        if !status.is_success() {
            return Err(RuntimeError::Rejected(raw));
        }
        serde_json::from_str(&raw).map_err(|error| RuntimeError::Unavailable(error.to_string()))
    }
}

#[async_trait]
impl RuntimeAdapter for HttpRunner {
    fn name(&self) -> &'static str {
        "docker-development"
    }

    fn network(&self) -> &str {
        &self.network
    }

    async fn health(&self) -> Result<(), RuntimeError> {
        let url = format!("{}/healthz", self.base_url);
        self.client
            .get(url)
            .send()
            .await
            .map_err(|error| RuntimeError::Unavailable(error.to_string()))?
            .error_for_status()
            .map_err(|error| RuntimeError::Unavailable(error.to_string()))?;
        Ok(())
    }

    async fn create(&self, spec: SandboxSpec) -> Result<RuntimeFacts, RuntimeError> {
        let environment: std::collections::BTreeMap<_, _> = spec.environment.into_iter().collect();
        let payload = serde_json::json!({
            "name": spec.name,
            "image": spec.image,
            "command": spec.command,
            "runtime": runtime_for(&spec.egress_mode),
            "pack_key": spec.pack_key,
            "tool_profile": spec.tool_profile,
            "environment": environment,
            "memory_limit": spec.memory_limit,
            "cpu_limit": spec.cpu_limit,
            "pids_limit": spec.pids_limit,
            "mounts": spec.mounts,
            "gpu_count": spec.gpu_count,
            "egress_mode": spec.egress_mode,
        });
        let facts: RunnerSandbox = self
            .send(reqwest::Method::POST, "/v1/sandboxes", Some(&payload))
            .await?;
        Ok(facts.into())
    }

    async fn inspect(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        let facts: RunnerSandbox = self
            .send(reqwest::Method::GET, &format!("/v1/sandboxes/{name}"), None)
            .await?;
        Ok(facts.into())
    }

    async fn list(&self) -> Result<Vec<RuntimeFacts>, RuntimeError> {
        let items: Vec<RunnerSandbox> = self
            .send(reqwest::Method::GET, "/v1/sandboxes", None)
            .await?;
        Ok(items.into_iter().map(Into::into).collect())
    }

    async fn start(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.action(name, "start").await
    }

    async fn stop(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.action(name, "stop").await
    }

    async fn restart(&self, name: &str) -> Result<RuntimeFacts, RuntimeError> {
        self.action(name, "restart").await
    }

    async fn remove(&self, name: &str) -> Result<(), RuntimeError> {
        let url = format!("{}/v1/sandboxes/{name}/remove", self.base_url);
        let mut request = self.client.post(url);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|error| RuntimeError::Unavailable(error.to_string()))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(RuntimeError::NotFound(name.to_string()));
        }
        response
            .error_for_status()
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        Ok(())
    }

    async fn logs(&self, name: &str, tail: u32) -> Result<String, RuntimeError> {
        #[derive(Deserialize)]
        struct LogResponse {
            #[serde(default)]
            logs: String,
        }
        let response: LogResponse = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/sandboxes/{name}/logs?tail={tail}"),
                None,
            )
            .await?;
        Ok(response.logs)
    }

    async fn exec(
        &self,
        name: &str,
        command: &[String],
        timeout_seconds: u32,
    ) -> Result<ExecResult, RuntimeError> {
        let payload = serde_json::json!({
            "command": command,
            "timeout_seconds": timeout_seconds.clamp(1, 600),
        });
        let result: ExecResult = self
            .send(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{name}/exec"),
                Some(&payload),
            )
            .await?;
        Ok(result)
    }

    async fn remote_ssh_run(
        &self,
        request: crate::domain::runtime::RemoteSshRun,
    ) -> Result<ExecResult, RuntimeError> {
        let payload = serde_json::json!({
            "host": request.host,
            "port": request.port,
            "user": request.user,
            "password": request.password,
            "private_key": request.private_key,
            "script": request.script,
            "env": request.env,
            "timeout_seconds": request.timeout_seconds,
        });
        self.send(reqwest::Method::POST, "/v1/remote/ssh-run", Some(&payload))
            .await
    }

    async fn hardware(&self) -> Result<HardwareReport, RuntimeError> {
        self.send(reqwest::Method::GET, "/v1/hardware", None).await
    }

    async fn build(&self, request: BuildRequest) -> Result<BuildResult, RuntimeError> {
        let payload = serde_json::to_value(&request)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        self.send_with_timeout(
            reqwest::Method::POST,
            "/v1/builds",
            Some(&payload),
            BUILD_TIMEOUT,
        )
        .await
    }

    async fn catalogue(&self, request: CatalogueRequest) -> Result<CatalogueAnswer, RuntimeError> {
        let payload = serde_json::to_value(&request)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        self.send(reqwest::Method::POST, "/v1/catalogue", Some(&payload))
            .await
    }

    async fn verify_image(
        &self,
        request: ImageVerification,
    ) -> Result<VerificationAnswer, RuntimeError> {
        let payload = serde_json::to_value(&request)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        self.send(reqwest::Method::POST, "/v1/images/verify", Some(&payload))
            .await
    }

    async fn live_view(&self, name: &str, kind: &str) -> Result<serde_json::Value, RuntimeError> {
        self.send(
            reqwest::Method::GET,
            &format!("/v1/sandboxes/{name}/live/{kind}"),
            None,
        )
        .await
    }

    async fn processes(&self, name: &str) -> Result<ProcessTable, RuntimeError> {
        self.send(
            reqwest::Method::GET,
            &format!("/v1/sandboxes/{name}/processes"),
            None,
        )
        .await
    }

    async fn process_action(&self, name: &str, pid: i64, action: &str, nice: Option<i8>) -> Result<(), RuntimeError> {
        let payload = serde_json::json!({"action": action, "nice": nice});
        let _: serde_json::Value = self.send(reqwest::Method::POST, &format!("/v1/sandboxes/{name}/processes/{pid}/action"), Some(&payload)).await?;
        Ok(())
    }

    async fn inventory(&self, name: &str) -> Result<InventoryProbe, RuntimeError> {
        /// The runner's envelope: its own name for the sandbox, the moment, the
        /// sections, and why a snapshot may be partial.
        #[derive(Deserialize)]
        struct RunnerInventory {
            taken_at: DateTime<Utc>,
            inventory: serde_json::Value,
            #[serde(default)]
            note: Option<String>,
        }

        let answer: RunnerInventory = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/sandboxes/{name}/inventory"),
                None,
            )
            .await?;

        Ok(InventoryProbe {
            taken_at: answer.taken_at,
            facts: answer.inventory,
            note: answer.note,
        })
    }

    async fn download_model(
        &self,
        request: ModelDownload,
    ) -> Result<ModelDownloadResult, RuntimeError> {
        let payload = serde_json::to_value(&request)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        self.send(reqwest::Method::POST, "/v1/models/download", Some(&payload))
            .await
    }

    async fn model_logs(&self, model_key: &str, tail: u32) -> Result<String, RuntimeError> {
        #[derive(Deserialize)]
        struct LogResponse {
            #[serde(default)]
            logs: String,
        }
        let response: LogResponse = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/models/{model_key}/logs?tail={tail}"),
                None,
            )
            .await?;
        Ok(response.logs)
    }

    async fn model_metrics(
        &self,
        model_key: &str,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.send::<serde_json::Value>(
            reqwest::Method::GET,
            &format!("/v1/models/{model_key}/metrics"),
            None,
        )
        .await
    }

    async fn install_libraries(
        &self,
        component: &str,
        images: &[String],
    ) -> Result<Vec<String>, RuntimeError> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            component: &'a str,
            images: &'a [String],
        }
        let payload = serde_json::to_value(Body { component, images })
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        let answer: serde_json::Value = self
            .send(reqwest::Method::POST, "/v1/cudax/install", Some(&payload))
            .await?;
        Ok(answer
            .get("pulled")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn serve_model(&self, request: ModelServe) -> Result<ModelServeResult, RuntimeError> {
        let payload = serde_json::to_value(&request)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        self.send(reqwest::Method::POST, "/v1/models/serve", Some(&payload))
            .await
    }

    async fn stop_model(&self, model_key: &str) -> Result<(), RuntimeError> {
        // The runner answers 404 when the container is already gone. That is
        // the state the caller wants, so it is not an error here.
        match self
            .send::<serde_json::Value>(
                reqwest::Method::DELETE,
                &format!("/v1/models/serve/{model_key}"),
                None,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(RuntimeError::NotFound(_)) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn set_forwards(
        &self,
        name: &str,
        forwards: &[ForwardRequest],
        authorized_keys: &[String],
        credentials: &[SshCredential],
    ) -> Result<Vec<ForwardState>, RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            forwards: Vec<ForwardState>,
        }
        let payload = serde_json::json!({
            "forwards": forwards,
            "authorized_keys": authorized_keys,
            "credentials": credentials,
        });
        let answer: Answer = self
            .send(reqwest::Method::PUT, &format!("/v1/sandboxes/{name}/forwards"), Some(&payload))
            .await?;
        Ok(answer.forwards)
    }

    async fn forward_states(&self, name: &str) -> Result<Vec<ForwardState>, RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            forwards: Vec<ForwardState>,
        }
        let answer: Answer = self
            .send(reqwest::Method::GET, &format!("/v1/sandboxes/{name}/forwards"), None)
            .await?;
        Ok(answer.forwards)
    }

    async fn clear_forwards(&self, name: &str) -> Result<(), RuntimeError> {
        let _: serde_json::Value = self
            .send(reqwest::Method::DELETE, &format!("/v1/sandboxes/{name}/forwards"), None)
            .await?;
        Ok(())
    }

    async fn host_load(&self) -> Result<serde_json::Value, RuntimeError> {
        self.send(reqwest::Method::GET, "/v1/host/load", None).await
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
        let payload = serde_json::json!({
            "memory_bytes": memory_bytes,
            "memory_reservation_bytes": memory_reservation_bytes,
            "cpus": cpus,
            "cpu_shares": cpu_shares,
            "pids_limit": pids_limit,
        });
        let _: serde_json::Value = self
            .send(reqwest::Method::PATCH, &format!("/v1/sandboxes/{name}/resources"), Some(&payload))
            .await?;
        Ok(())
    }

    async fn backup_create(&self, root_path: &str, backup_id: &str) -> Result<(i64, String), RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            bytes: i64,
            sha256: String,
        }
        let payload = serde_json::json!({ "root_path": root_path, "backup_id": backup_id });
        let answer: Answer = self.send(reqwest::Method::POST, "/v1/backups", Some(&payload)).await?;
        Ok((answer.bytes, answer.sha256))
    }

    async fn backup_restore(
        &self,
        backup_id: &str,
        root_path: &str,
        sha256: &str,
        replace: bool,
    ) -> Result<serde_json::Value, RuntimeError> {
        let payload = serde_json::json!({ "root_path": root_path, "sha256": sha256, "replace": replace });
        self.send(reqwest::Method::POST, &format!("/v1/backups/{backup_id}/restore"), Some(&payload)).await
    }

    async fn backup_delete(&self, backup_id: &str) -> Result<(), RuntimeError> {
        let _: serde_json::Value = self
            .send(reqwest::Method::DELETE, &format!("/v1/backups/{backup_id}"), None)
            .await?;
        Ok(())
    }

    async fn openshell_view(&self, name: &str) -> Result<serde_json::Value, RuntimeError> {
        self.send(reqwest::Method::GET, &format!("/v1/sandboxes/{name}/openshell"), None).await
    }

    async fn openshell_events(&self, since_seconds: u32) -> Result<Vec<serde_json::Value>, RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            events: Vec<serde_json::Value>,
        }
        let answer: Answer = self
            .send(reqwest::Method::GET, &format!("/v1/openshell/events?since={since_seconds}"), None)
            .await?;
        Ok(answer.events)
    }

    async fn set_openshell_policy(
        &self,
        name: &str,
        document: &serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let payload = serde_json::json!({ "document": document });
        let _: serde_json::Value = self
            .send(reqwest::Method::PUT, &format!("/v1/sandboxes/{name}/openshell-policy"), Some(&payload))
            .await?;
        Ok(())
    }

    async fn components(&self) -> Result<Vec<ComponentState>, RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            components: Vec<ComponentState>,
        }
        let answer: Answer = self.send(reqwest::Method::GET, "/v1/system/components", None).await?;
        Ok(answer.components)
    }

    async fn set_component_limits(
        &self,
        service: &str,
        limits: ComponentLimits,
    ) -> Result<(), RuntimeError> {
        if limits.is_empty() {
            return Err(RuntimeError::Rejected("nothing to change".into()));
        }
        let payload = serde_json::to_value(&limits)
            .map_err(|error| RuntimeError::Rejected(error.to_string()))?;
        let _: serde_json::Value = self
            .send(
                reqwest::Method::PUT,
                &format!("/v1/system/components/{service}/limits"),
                Some(&payload),
            )
            .await?;
        Ok(())
    }

    async fn restart_component(&self, service: &str) -> Result<(), RuntimeError> {
        let _: serde_json::Value = self
            .send(
                reqwest::Method::POST,
                &format!("/v1/system/components/{service}/restart"),
                Some(&serde_json::json!({})),
            )
            .await?;
        Ok(())
    }

    async fn component_logs(&self, service: &str, tail: u32) -> Result<String, RuntimeError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            logs: String,
        }
        let answer: Answer = self
            .send(
                reqwest::Method::GET,
                &format!("/v1/system/components/{service}/logs?tail={tail}"),
                None,
            )
            .await?;
        Ok(answer.logs)
    }

    async fn snapshot_backup(
        &self,
        repo: &str,
        password: &str,
        path: &str,
        tags: &[String],
    ) -> Result<crate::domain::runtime::SnapshotFact, RuntimeError> {
        let body = serde_json::json!({ "repo": repo, "password": password, "path": path, "tags": tags });
        self.send(reqwest::Method::POST, "/v1/snapshots/backup", Some(&body)).await
    }

    async fn snapshot_restore(
        &self,
        repo: &str,
        password: &str,
        snapshot: &str,
        source_path: &str,
        path: &str,
    ) -> Result<serde_json::Value, RuntimeError> {
        let body = serde_json::json!({
            "repo": repo, "password": password, "snapshot": snapshot, "source_path": source_path, "path": path
        });
        self.send(reqwest::Method::POST, "/v1/snapshots/restore", Some(&body)).await
    }

    async fn prune_snapshots(&self, name: &str, password: &str, keep_last: i32, keep_daily: i32) -> Result<serde_json::Value, RuntimeError> {
        let body = serde_json::json!({ "name": name, "password": password, "keep_last": keep_last, "keep_daily": keep_daily });
        self.send(reqwest::Method::POST, "/v1/snapshots/prune", Some(&body)).await
    }

    async fn images(&self) -> Result<Vec<crate::domain::runtime::NodeImageFact>, RuntimeError> {
        #[derive(Deserialize)]
        struct Listing {
            items: Vec<crate::domain::runtime::NodeImageFact>,
        }
        let listing: Listing = self.send(reqwest::Method::GET, "/v1/images", None).await?;
        Ok(listing.items)
    }

    async fn pull_image(&self, image: &str) -> Result<crate::domain::runtime::NodeImageFact, RuntimeError> {
        let body = serde_json::json!({ "image": image });
        self.send(reqwest::Method::POST, "/v1/images/pull", Some(&body)).await
    }

    async fn open_vnc(&self, name: &str, view_only: bool) -> Result<super::VncStream, RuntimeError> {
        // The runner drops keyboard, pointer and clipboard messages when asked
        // for a view-only stream, so the bound is enforced server side.
        let suffix = if view_only { "?view=1" } else { "" };
        let url = format!("{}/v1/sandboxes/{}/vnc{suffix}", self.ws_base_url(), name);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RuntimeError::Rejected(format!("invalid desktop URL: {error}")))?;
        // Same rule as the terminal: the node token rides in the subprotocol
        // header, never in a URL that a log would keep.
        let protocol = match self.node_token() {
            Some(token) => format!("nestbox-vnc, {token}"),
            None => "nestbox-vnc".to_string(),
        };
        request.headers_mut().insert(
            "sec-websocket-protocol",
            reqwest::header::HeaderValue::from_str(&protocol).map_err(|_| {
                RuntimeError::Rejected("the node token is not a valid header value".to_string())
            })?,
        );
        let upgrade = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("the runner did not answer the upgrade".into()))?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => match response.status().as_u16() {
                404 => RuntimeError::NotFound(name.to_string()),
                401 | 403 => RuntimeError::Rejected("the runner refused the node token".to_string()),
                status => RuntimeError::Rejected(format!("the runner refused the desktop ({status})")),
            },
            other => RuntimeError::Unavailable(other.to_string()),
        })?;
        Ok(upgrade.0)
    }

    async fn terminal_sessions(&self, name: &str) -> Result<serde_json::Value, RuntimeError> {
        self.send(reqwest::Method::GET, &format!("/v1/sandboxes/{name}/terminal/sessions"), None).await
    }

    async fn open_terminal(&self, name: &str, tab_id: &str, who: &str) -> Result<BoxTerminal, RuntimeError> {
        // `tab_id` is validated to ASCII alphanumerics, `_`, and `-` before it
        // reaches the runtime, so it is already safe as this path's query value.
        // `who` is a label; it is percent-encoded so a name with a space or an
        // accent cannot break the query.
        let who_encoded: String = who
            .bytes()
            .map(|byte| match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
                other => format!("%{other:02X}"),
            })
            .collect();
        let url = format!("{}/v1/sandboxes/{}/terminal?tab={tab_id}&who={who_encoded}", self.ws_base_url(), name);
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RuntimeError::Rejected(format!("invalid terminal URL: {error}")))?;
        // The token travels in a header, not in a query string: a URL ends up in
        // access logs and in every error message that quotes the request. The
        // subprotocol the runner answers with stays first, so the negotiation
        // still selects it.
        // Resolved per terminal, not cached at start (ADR 0016): rotating the
        // file takes effect for the next session instead of requiring a restart
        // of the API, and a token never sits in the process longer than the
        // handshake that needs it.
        let token = self.node_token();
        let protocol = match &token {
            Some(token) => format!("{TERMINAL_SUBPROTOCOL}, {token}"),
            None => TERMINAL_SUBPROTOCOL.to_string(),
        };
        request.headers_mut().insert(
            "sec-websocket-protocol",
            reqwest::header::HeaderValue::from_str(&protocol).map_err(|_| {
                RuntimeError::Rejected("the node token is not a valid header value".to_string())
            })?,
        );
        let upgrade = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("the runner did not answer the upgrade".into()))?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                let status = response.status().as_u16();
                match status {
                    404 => RuntimeError::NotFound(name.to_string()),
                    // The runner demands the node token, and the message names
                    // the variable: a terminal that silently refuses to open is
                    // the kind of failure that costs an afternoon.
                    401 | 403 => RuntimeError::Rejected(
                        "the runner refused the node token; NESTBOX_NODE_TOKEN must be the \
                         same on the control API and on the sandbox runner"
                            .to_string(),
                    ),
                    // The runner's handshake parser refuses a malformed
                    // `Sec-WebSocket-Protocol` before the endpoint sees it, so a
                    // token with `+`, `/` or `=` arrives here as a bare 400.
                    400 => RuntimeError::Rejected(
                        "the runner rejected the terminal handshake (400); a node token must \
                         be a single token of letters and digits — hex, not base64"
                            .to_string(),
                    ),
                    _ => RuntimeError::Rejected(format!(
                        "the runner refused the terminal ({status})"
                    )),
                }
            }
            other => RuntimeError::Unavailable(other.to_string()),
        })?;
        Ok(Box::new(RunnerTerminal::new(upgrade.0)))
    }
}

/// Which runtime a sandbox with this egress mode runs on.
///
/// The gateway decides every connection itself and parses anything sent to another
/// proxy as HTTP, so a `CONNECT` to the egress proxy is refused there. Reading TLS
/// (chats, token use, method and path rules) therefore needs the Docker runtime;
/// `direct` needs a route of its own, which a gateway sandbox does not have. Only
/// the mode that just allows or blocks by host — `forward_proxy` — runs on the
/// gateway, which is also the one whose decisions the gateway already makes.
pub fn runtime_for(egress_mode: &str) -> String {
    let openshell_default = std::env::var("NESTBOX_SANDBOX_RUNTIME")
        .ok()
        .filter(|value| value == "openshell")
        .is_some();
    runtime_choice(openshell_default, egress_mode).to_string()
}

/// The decision itself, so it can be tested without touching the process environment.
fn runtime_choice(openshell_default: bool, egress_mode: &str) -> &'static str {
    if openshell_default && !matches!(egress_mode, "direct" | "transparent" | "hybrid") {
        "openshell"
    } else {
        "docker-development"
    }
}

/// The subprotocol the terminal speaks. Named so both ends agree, and so a
/// proxy that inspects the upgrade sees a protocol rather than a bare socket.
const TERMINAL_SUBPROTOCOL: &str = "nestbox-terminal";

/// The node token, from a file when one is configured and from the environment
/// otherwise.
///
/// The file wins because an environment variable is visible in `docker inspect`
/// and in a crash dump (ADR 0005), and this token is the only thing between a
/// compromised sandbox and a shell in its neighbour.
fn read_node_token() -> Option<String> {
    if let Ok(path) = std::env::var("NESTBOX_NODE_TOKEN_FILE") {
        return match std::fs::read_to_string(&path) {
            Ok(value) => Some(value.trim().to_string()).filter(|token| !token.is_empty()),
            Err(error) => {
                tracing::warn!(path = %path, error = %error, "cannot read the node token file");
                None
            }
        };
    }
    std::env::var("NESTBOX_NODE_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|token| !token.is_empty())
}

impl HttpRunner {
    async fn action(&self, name: &str, action: &str) -> Result<RuntimeFacts, RuntimeError> {
        let facts: RunnerSandbox = self
            .send(
                reqwest::Method::POST,
                &format!("/v1/sandboxes/{name}/{action}"),
                None,
            )
            .await?;
        Ok(facts.into())
    }
}

#[cfg(test)]
mod runtime_choice_tests {
    use super::runtime_choice;

    #[test]
    fn only_the_host_only_mode_stays_on_the_gateway() {
        assert_eq!(runtime_choice(true, "forward_proxy"), "openshell");
        for mode in ["transparent", "hybrid", "direct"] {
            assert_eq!(runtime_choice(true, mode), "docker-development", "{mode}");
        }
        assert_eq!(runtime_choice(false, "forward_proxy"), "docker-development");
    }

    #[test]
    fn a_build_is_allowed_to_take_longer_than_a_normal_call() {
        // The desktop image needs about eleven minutes; the bound every other
        // call shares is shorter than that, which is why a build carries its
        // own. Sharing it again would fail a real build while the runner works.
        assert!(super::BUILD_TIMEOUT > super::CLIENT_TIMEOUT);
    }
}

// (rebuild marker: the runtime is chosen per egress mode)
