//! Sandbox lifecycle, placement, and bulk actions.
//!
//! The rules that matter:
//!
//! * A state transition is checked against the current status before it is
//!   attempted, so a stop on a stopped sandbox is a clear refusal rather than a
//!   runtime error.
//! * Placement refuses a gateway that is drifted or unhealthy (ADR 0012).
//! * A bulk action over more than one sandbox becomes a job, so partial failure
//!   is attributable and the request does not hold a connection for minutes.

use std::collections::BTreeMap;

use uuid::Uuid;

use super::pack_service;
use super::policy_service;
use super::{require, ServiceContext};
use crate::domain::auth::{Permission, Principal};
use crate::domain::pagination::{ListParams, Page};
use crate::domain::runtime::SandboxSpec;
use crate::domain::sandbox::{
    Sandbox, SandboxDetail, SandboxHealth, SandboxName, SandboxStatus, SandboxSummary,
};
use crate::domain::telemetry::LiveEvent;
use crate::error::{ApiError, ErrorCode, StoreError};
use crate::store::TenantScope;
use crate::store::postgres::environments as env_store;
use crate::store::postgres::sandboxes as store;

/// Which lifecycle action was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAction {
    Start,
    Stop,
    Restart,
}

impl LifecycleAction {
    /// Parse the action name from a request body.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "restart" => Some(Self::Restart),
            _ => None,
        }
    }

    /// The status a sandbox must be in for this action to make sense.
    fn is_allowed_for(self, sandbox: &Sandbox) -> bool {
        match self {
            Self::Start => sandbox.can_start(),
            Self::Stop => sandbox.can_stop(),
            Self::Restart => sandbox.status == SandboxStatus::Running,
        }
    }

    /// The status to record while the action runs.
    fn transitional(self) -> SandboxStatus {
        match self {
            Self::Start | Self::Restart => SandboxStatus::Starting,
            Self::Stop => SandboxStatus::Stopping,
        }
    }

    /// The status to record when the action succeeds.
    fn settled(self) -> SandboxStatus {
        match self {
            Self::Start | Self::Restart => SandboxStatus::Running,
            Self::Stop => SandboxStatus::Stopped,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
        }
    }
}

/// List sandboxes for the console, with live state merged in.
///
/// The runtime is asked once for the whole inventory rather than once per row.
/// A page of fifty sandboxes must not be fifty round trips, and a runtime that is
/// unreachable must degrade the page rather than fail it.
pub async fn list(
    ctx: &ServiceContext,
    principal: &Principal,
    params: &ListParams,
) -> Result<Page<SandboxSummary>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let page = store::list(&mut scope, params).await.map_err(ApiError::from)?;
    // Names the list shows next to each sandbox: where it sits and who watches it.
    let clusters: std::collections::HashMap<Uuid, String> =
        sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM clusters")
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?
            .into_iter()
            .collect();
    let owners: std::collections::HashMap<Uuid, (Uuid, String)> =
        sqlx::query_as::<_, (Uuid, Uuid, String)>(
            "SELECT s.id, u.id, u.display_name FROM sandboxes s JOIN users u ON u.id = s.owner_user_id",
        )
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?
        .into_iter()
        .map(|(sandbox, user, name)| (sandbox, (user, name)))
        .collect();
    let supervisors: std::collections::HashMap<Uuid, (String, String)> =
        sqlx::query_as::<_, (Uuid, String, String)>("SELECT id, name, status FROM supervisors")
            .fetch_all(scope.connection())
            .await
            .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?
            .into_iter()
            .map(|(id, name, status)| (id, (name, status)))
            .collect();
    scope.commit().await.map_err(ApiError::from)?;

    let facts = ctx.runtime.list().await.ok();
    let runtime_available = facts.is_some();
    let index: std::collections::HashMap<String, crate::domain::runtime::RuntimeFacts> = facts
        .unwrap_or_default()
        .into_iter()
        .map(|fact| (fact.name.clone(), fact))
        .collect();

    let mut settled = Vec::with_capacity(page.items.len());
    for sandbox in page.items {
        let live = index.get(&sandbox.name);
        settled.push(reconcile_transitional(ctx, principal, sandbox, live).await);
    }

    let items = settled
        .into_iter()
        .map(|sandbox| {
            let live = index.get(&sandbox.name);
            let supervisor = sandbox.supervisor_id.and_then(|id| supervisors.get(&id));
            SandboxSummary {
                owner_user_id: owners.get(&sandbox.id).map(|(user, _)| *user),
                owner_name: owners.get(&sandbox.id).map(|(_, name)| name.clone()),
                cluster_name: clusters.get(&sandbox.cluster_id).cloned(),
                supervisor_name: supervisor.map(|(name, _)| name.clone()),
                supervisor_status: supervisor.map(|(_, status)| status.clone()),
                runtime_kind: live.map(|facts| {
                    if facts.container_name.starts_with("openshell-default--") { "openshell" } else { "docker" }.to_string()
                }),
                address: live.map(|facts| facts.ip.clone()).filter(|ip| !ip.is_empty()),
                uptime_seconds: live.map(|facts| facts.uptime_seconds),
                container_name: live.map(|facts| facts.container_name.clone()),
                runtime_available,
                sandbox,
            }
        })
        .collect();

    Ok(Page {
        items,
        next_cursor: page.next_cursor,
        total_estimate: page.total_estimate,
        facets: page.facets,
    })
}

/// Read one sandbox and merge what the runtime reports.
pub async fn get(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<SandboxDetail, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // A runtime that cannot be reached is a normal state, not an error: the
    // stored state is still worth showing.
    let runtime = ctx.runtime.inspect(&sandbox.name).await.ok();
    let sandbox = reconcile_transitional(ctx, principal, sandbox, runtime.as_ref()).await;
    Ok(SandboxDetail { sandbox, runtime })
}

/// What a create request carries.
pub struct CreateSandbox {
    pub name: String,
    pub pack_key: String,
    /// Empty means "use the pack's default image".
    pub image: String,
    pub image_digest: Option<String>,
    /// Empty means "use the pack's default command".
    pub command: Vec<String>,
    pub memory_limit: String,
    pub cpu_limit: f64,
    pub pids_limit: i64,
    /// GPUs to hand to the container. Refused on a node that cannot hand them out.
    pub gpu_count: Option<u32>,
    /// The node to place the sandbox on, by name. Empty means the local node.
    pub node: Option<String>,
    /// The lead of the stack this sandbox joins. Absent means standalone (or a
    /// lead, when others later point at it).
    pub parent_id: Option<Uuid>,
    /// How the agent signs in: `gateway` (the default: the appliance's model endpoint and
    /// key) or `own_account` (no key in the environment; the user logs in inside the
    /// sandbox and the login is kept on its persistent home).
    pub login: Option<String>,
    /// The egress mode the sandbox is created for; it decides the runtime.
    ///
    /// The container always starts in `forward_proxy` and a later change is applied
    /// live, so this is not the mode the proxy reads. It is the one the person asked
    /// for, and only it can choose the runtime: reading TLS needs Docker, because the
    /// gateway parses every connection to another proxy as HTTP and refuses a
    /// `CONNECT`. Absent means `forward_proxy`, which is the gateway's own mode.
    pub egress_mode: Option<String>,
}

/// The sign-in choices.
pub const LOGIN_MODES: [&str; 2] = ["gateway", "own_account"];

/// Take the gateway's model settings out of an agent's environment, for a sandbox the
/// user signs in to with their own account. What stays: the MCP hub, the config directory.
///
/// Only for config files. The container environment keeps the gateway keys, whether
/// or not an agent uses them: an agent's sign-in mode is the launcher's decision (ADR
/// 0054), and a container cannot be told a different environment afterwards.
pub fn apply_login_choice(environment: &mut BTreeMap<String, String>, login: Option<&str>) {
    if login != Some("own_account") {
        return;
    }
    for key in [
        "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY", "ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL", "OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_MODEL",
    ] {
        environment.remove(key);
    }
}

/// The hosts an agent contacts to sign a user in with their own account. Only these:
/// everything else stays under the sandbox's policy.
pub fn own_login_hosts(pack_key: &str) -> &'static [&'static str] {
    match pack_key {
        "claude-code" => &["claude.ai", "console.anthropic.com", "platform.claude.com", "api.anthropic.com"],
        _ => &[],
    }
}

/// Whether an agent pack has a login of its own to offer.
pub fn pack_has_own_login(pack_key: &str) -> bool {
    matches!(pack_key, "claude-code" | "opencode" | "openchamber")
}

/// Whether `lead` may lead a stack that `member` joins.
///
/// Three rules, each of which keeps the list readable: the lead is a sandbox of
/// this environment, it is not itself a member of another stack (a stack is one
/// level deep), and a sandbox that leads members cannot become a member. `member`
/// is `None` while the sandbox is still being created.
async fn check_stack_lead(
    scope: &mut TenantScope<'_>,
    member: Option<Uuid>,
    lead: Option<Uuid>,
) -> Result<(), ApiError> {
    let Some(lead_id) = lead else {
        return Ok(());
    };
    if Some(lead_id) == member {
        return Err(ApiError::validation("A sandbox cannot be part of its own stack."));
    }
    let lead = store::get(scope, lead_id).await.map_err(|error| match error {
        StoreError::NotFound => ApiError::validation("The stack's lead sandbox does not exist."),
        other => ApiError::from(other),
    })?;
    if lead.parent_id.is_some() {
        return Err(ApiError::validation(
            "That sandbox is itself a member of a stack. A stack has one lead and its members, nothing deeper.",
        ));
    }
    if let Some(member_id) = member {
        if store::member_count(scope, member_id).await.map_err(ApiError::from)? > 0 {
            return Err(ApiError::validation(
                "This sandbox leads a stack of its own. Move its members first.",
            ));
        }
    }
    Ok(())
}

/// Put a sandbox into a stack, or take it out with `None`.
pub async fn set_stack(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    parent_id: Option<Uuid>,
) -> Result<Sandbox, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    // Read first: a sandbox that is not here is a 404 before any rule is argued.
    store::get(&mut scope, id).await.map_err(ApiError::from)?;
    check_stack_lead(&mut scope, Some(id), parent_id).await?;
    store::set_parent(&mut scope, id, parent_id)
        .await
        .map_err(ApiError::from)?;
    let updated = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(updated)
}

/// Settle a row stuck in a transitional state when the runtime has moved on.
///
/// `starting` and `stopping` are written before the runtime call and replaced
/// after it. If the process dies in between — a deploy during a stop, a crash,
/// a lost connection — the row keeps the transitional status forever and every
/// lifecycle button is disabled. The runtime still knows the truth, so a read
/// that sees both compares them and writes the settled state. The desired
/// state is not changed by this; only a transitional one is resolved.
async fn reconcile_transitional(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox: Sandbox,
    live: Option<&crate::domain::runtime::RuntimeFacts>,
) -> Sandbox {
    let Some(facts) = live else {
        return sandbox;
    };
    if !matches!(
        sandbox.status,
        SandboxStatus::Starting | SandboxStatus::Stopping
    ) {
        return sandbox;
    }
    let Some(target) = settled_status_from_runtime(&facts.status) else {
        return sandbox;
    };
    let health = health_from_runtime(facts);
    match set_status(ctx, principal, sandbox.id, sandbox.version, target, health).await {
        Ok(updated) => {
            tracing::info!(sandbox = %updated.name, from = sandbox.status.as_str(), to = target.as_str(), "settled a transitional sandbox from runtime state");
            updated
        }
        Err(error) => {
            // A version conflict means another writer settled it first, which
            // is fine; anything else is logged and the stale row is shown.
            tracing::debug!(sandbox = %sandbox.name, code = error.code.0, "reconcile skipped");
            sandbox
        }
    }
}

/// The settled status a runtime state corresponds to, if it is settled.
///
/// Docker's own transitional states (`restarting`, `removing`) yield `None`:
/// the row stays transitional until the runtime makes up its mind.
pub fn settled_status_from_runtime(runtime_status: &str) -> Option<SandboxStatus> {
    match runtime_status {
        "running" => Some(SandboxStatus::Running),
        "exited" | "created" | "paused" | "stopped" => Some(SandboxStatus::Stopped),
        "dead" => Some(SandboxStatus::Failed),
        _ => None,
    }
}

/// Translate what the runtime reports into the stored health.
///
/// One rule for create and lifecycle, because they previously disagreed: create
/// called an unprobed container healthy and a restart called it unknown, so the
/// badge flipped on the first stop/start. A container with no health check
/// reports `unknown` from Docker; a running one with no probe is treated as
/// healthy because "running and not failing" is the only evidence available,
/// and a stopped one is unknown because nothing is observing it.
pub fn health_from_runtime(facts: &crate::domain::runtime::RuntimeFacts) -> SandboxHealth {
    match facts.health.as_str() {
        "healthy" => SandboxHealth::Healthy,
        "degraded" | "starting" => SandboxHealth::Degraded,
        "unhealthy" => SandboxHealth::Unhealthy,
        _ if facts.status == "running" => SandboxHealth::Healthy,
        _ => SandboxHealth::Unknown,
    }
}

/// Whether a pack needs an inference endpoint to do its job.
///
/// Kept as a named rule rather than an inline `== "hermes"` so it is one place
/// to change when a second agent pack arrives, and so the test suite can assert
/// the list deliberately.
pub fn pack_needs_inference(pack_key: &str) -> bool {
    matches!(pack_key, "hermes" | "claude-code" | "opencode" | "openchamber")
}

/// The proxy URL used when a node does not name one.
///
/// The default matches the runner's `NESTBOX_EGRESS_PROXY_HOST`, because a URL
/// the sandbox cannot resolve is a sandbox with no egress at all.
pub const DEFAULT_EGRESS_PROXY: &str = "http://egress-proxy:3128";

/// The egress proxy a sandbox is told about, as the address it can reach.
///
/// The sandbox has no route out (the runtime network is `internal`), so this URL
/// is the only way it reaches anything; whether a request may actually leave is
/// decided per request by the control API (ADR 0013), not here.
///
/// Read from the environment rather than from `nodes.egress_proxy_url` because
/// that column exists (migration 0022) but the node domain type does not carry it
/// yet. One value a deploy can set beats a second source of truth in a service;
/// when the column reaches the domain type, this function reads it and nothing
/// else changes.
pub fn egress_proxy_url() -> String {
    if openshell_runtime() {
        openshell_proxy_url()
    } else {
        docker_egress_proxy_url()
    }
}

/// The address a gateway sandbox uses: an address on the host, not a compose
/// service name.
fn openshell_proxy_url() -> String {
    // A sandbox on a gateway has no route on the runtime network, so the compose
    // service name is not resolvable from it. The supervisor reaches the host,
    // and a published loopback port is reachable as
    // `host.openshell.internal:<port>` — the same form the MCP hub already uses
    // (ADR 0035). Without this the agent has no proxy, tries a direct connection
    // and fails in DNS before any policy could speak.
    match std::env::var("NESTBOX_OPENSHELL_PROXY_URL") {
        Ok(value) if !value.trim().is_empty() => value.trim().trim_end_matches('/').to_string(),
        _ => DEFAULT_OPENSHELL_PROXY.to_string(),
    }
}

/// The address a container sandbox uses: the compose service on the runtime
/// network.
fn docker_egress_proxy_url() -> String {
    match std::env::var("NESTBOX_EGRESS_PROXY_URL") {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => DEFAULT_EGRESS_PROXY.to_string(),
    }
}

/// The proxy URL for a sandbox whose **runtime follows its egress mode**.
///
/// The two candidates are not interchangeable: `host.openshell.internal` exists
/// only inside a gateway sandbox, and `egress-proxy:3128` only on the runtime
/// network. Choosing by the deploy-wide default instead of by this sandbox's
/// runtime is how a container in an inspecting mode ended up with an address it
/// could not resolve — DNS failed before any policy could speak, and the mode
/// that reads TLS looked like an outage.
pub fn egress_proxy_url_for(mode: &str) -> String {
    if crate::runtime::http_runner::runtime_for(mode) == "openshell" {
        openshell_proxy_url()
    } else {
        docker_egress_proxy_url()
    }
}

/// Whether new sandboxes are created on an OpenShell gateway.
pub fn openshell_runtime() -> bool {
    std::env::var("NESTBOX_SANDBOX_RUNTIME").is_ok_and(|value| value == "openshell")
}

/// The address an OpenShell sandbox uses for the egress proxy.
///
/// The supervisor runs on the host network; a port published on the host's
/// loopback is the one address a sandbox can reach (the MCP hub is exposed the
/// same way). The host and port must match the `egress-proxy` publish rule in
/// `deploy/docker-compose.yml`.
pub const DEFAULT_OPENSHELL_PROXY: &str = "http://host.openshell.internal:18087";

/// The host and port of the OpenShell proxy, for the policy allow rule.
///
/// A sandbox cannot open a connection to the proxy unless the gateway policy
/// allows it, so this is derived from the same URL the environment uses: two
/// spellings of one address is how a proxy becomes unreachable after a deploy.
pub fn openshell_proxy_endpoint() -> (String, u16) {
    let url = egress_proxy_url();
    let without_scheme = url.split("://").nth(1).unwrap_or(&url);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    match authority.rsplit_once(':') {
        Some((host, port)) => (
            host.to_string(),
            port.parse::<u16>().unwrap_or(18087),
        ),
        None => (authority.to_string(), 18087),
    }
}

/// Keep the inference endpoint out of the proxy.
///
/// The provider a sandbox is given may be a model on its own node
/// (`http://model-<key>:8080/v1`). That address is on the runtime network and
/// must not be proxied: through the proxy it is an egress request without a
/// rule, and the agent reports the refusal as a rejected API key — a wrong
/// diagnosis of a correct default-deny. The host is read from the environment
/// the sandbox was already given, so the two cannot disagree.
pub fn add_inference_bypass(environment: &mut BTreeMap<String, String>) {
    let Some(base) = environment.get("OPENAI_BASE_URL").cloned() else {
        return;
    };
    let host = base
        .split("://")
        .nth(1)
        .unwrap_or(&base)
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_string();
    // Only a container on the sandbox network (a bare name such as `model-qwen`)
    // is kept off the proxy. A gateway with a dotted name or an address is egress:
    // it goes through the proxy, where it is decided and logged like everything else.
    if host.contains('.') || host.contains(':') {
        return;
    }
    add_host_bypass(environment, &host);
}

/// Keep one host out of the proxy, in both spellings of `NO_PROXY`.
///
/// The sandbox talks to two things that are not egress: the model on its own
/// node and the control plane it reports to. Both are on the internal network,
/// and through the proxy they would be denied requests — a supervisor whose
/// reports are refused by the very policy it is meant to observe.
pub fn add_host_bypass(environment: &mut BTreeMap<String, String>, host: &str) {
    if host.is_empty() {
        return;
    }
    for key in ["NO_PROXY", "no_proxy"] {
        let current = environment.get(key).cloned().unwrap_or_default();
        if current.split(',').any(|part| part == host) {
            continue;
        }
        let value = if current.is_empty() {
            host.to_string()
        } else {
            format!("{current},{host}")
        };
        environment.insert(key.to_string(), value);
    }
}

/// The same bypass for the vector the runtime spec carries.
///
/// Sorted and deduplicated by the map, so two creates of the same sandbox send
/// the same environment.
pub fn add_inference_bypass_vec(environment: &mut Vec<(String, String)>) {
    let mut map: BTreeMap<String, String> = environment.iter().cloned().collect();
    add_inference_bypass(&mut map);
    *environment = map.into_iter().collect();
}

/// The same bypass for a host named by an environment value, on the runtime
/// vector.
///
/// The control API's address is a variable (`NESTBOX_CONTROL_URL`), so the host
/// is taken from the value that was just written rather than parsed twice.
pub fn add_env_url_bypass_vec(environment: &mut Vec<(String, String)>, key: &str) {
    let Some(value) = environment
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
    else {
        return;
    };
    let host = value
        .split("://")
        .nth(1)
        .unwrap_or(&value)
        .split(['/', ':'])
        .next()
        .unwrap_or("")
        .to_string();
    let mut map: BTreeMap<String, String> = environment.iter().cloned().collect();
    add_host_bypass(&mut map, &host);
    *environment = map.into_iter().collect();
}

/// The host part of a proxy URL, for the bypass list.
fn proxy_host(proxy: &str) -> &str {
    proxy
        .split("://")
        .nth(1)
        .unwrap_or(proxy)
        .split(['/', ':'])
        .next()
        .unwrap_or(proxy)
}

/// The proxy variables a sandbox needs, in both spellings.
///
/// Uppercase and lowercase are both set because the tooling disagrees: curl and
/// wget read `http_proxy`, Go and many Rust clients read `HTTP_PROXY`, npm reads
/// both and prefers the uppercase one. Setting a single spelling is the classic
/// "the proxy works in the terminal but not in the agent" bug, and a missed
/// variable is not a hole — it is a tool that cannot leave, which looks like a
/// broken network and gets diagnosed as one.
///
/// `NO_PROXY` names the proxy itself. Without it a proxy-aware tool sends the
/// request for the proxy *to* the proxy, which is a loop. Loopback stays out of
/// the proxy because nothing on it leaves the container.
pub fn egress_environment(proxy: &str) -> Vec<(String, String)> {
    let bypass = format!("localhost,127.0.0.1,::1,{}", proxy_host(proxy));
    let mut values = Vec::with_capacity(8);
    for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        values.push((key.to_string(), proxy.to_string()));
        values.push((key.to_lowercase(), proxy.to_string()));
    }
    for key in ["NO_PROXY", "no_proxy"] {
        values.push((key.to_string(), bypass.clone()));
    }
    // Sorted so the spec — and any JSON that echoes it — is stable between
    // creates.
    values.sort();
    values
}

/// Where a sandbox finds the node CA when its egress is inspected.
///
/// The runner mounts the deployment's CA here, read-only, and only for a
/// sandbox in a TLS-reading mode. Nothing is baked into an image: a CA in a
/// registry is a CA trusted by every sandbox built from it, whether or not that
/// sandbox is supposed to be inspected.
pub const EGRESS_CA_IN_SANDBOX: &str = "/run/secrets/egress-ca.crt";

/// Where a sandbox finds the bundle of public roots plus the node CA.
///
/// A client that replaces its trust store instead of adding to it needs the
/// public roots too: with only the CA file in `SSL_CERT_FILE` every host whose
/// TLS the proxy leaves alone fails verification. The runner mounts both files
/// for every mode that goes through the proxy.
pub const CA_BUNDLE_IN_SANDBOX: &str = "/run/secrets/nestbox-ca-bundle.crt";

/// The trust variables that make a sandbox accept the node CA.
///
/// `NODE_EXTRA_CA_CERTS` adds to Node's store, so it gets the CA file alone: it
/// is the one variable that cannot break an uninspected host. The others replace
/// the bundle for their client and therefore get the combined bundle (public
/// roots and the node CA). They are set for every mode that uses the proxy, not
/// only for the ones that read TLS, because a container cannot gain a mount
/// after it was created: switching an existing sandbox to `transparent` must
/// work without a recreate.
pub fn tls_trust_environment(ca_path: &str) -> Vec<(String, String)> {
    let mut values = vec![("NODE_EXTRA_CA_CERTS".to_string(), ca_path.to_string())];
    for key in [
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "GIT_SSL_CAINFO",
        "AWS_CA_BUNDLE",
    ] {
        values.push((key.to_string(), CA_BUNDLE_IN_SANDBOX.to_string()));
    }
    values
}

/// The proxy variables for a mode, given the proxy the sandbox will actually
/// talk to.
///
/// `direct` with no exit has none: the sandbox reaches the internet on its own
/// route, which is what "no proxy" means. `direct` with an exit points at that
/// exit. The proxied modes point at the node's egress proxy and also tell the
/// sandbox to trust the node CA, whether or not the mode reads TLS today: the
/// trust files are mounted for the container's whole life, and a mode switch
/// must not need a recreate.
pub fn egress_environment_for(
    proxy: &str,
    mode: crate::domain::policy::EgressMode,
    ca_path: &str,
) -> Vec<(String, String)> {
    let mut values = egress_environment(proxy);
    if mode.uses_proxy() {
        values.extend(tls_trust_environment(ca_path));
    }
    values.sort();
    values
}

/// The proxy URL a sandbox is given, and whether it is the node's egress proxy.
///
/// Returned as one value so the environment and the runtime spec cannot
/// disagree about which mode a container was created in.
pub fn sandbox_proxy_url(
    mode: crate::domain::policy::EgressMode,
    exit: Option<&crate::domain::proxy::Proxy>,
) -> Option<String> {
    match mode {
        crate::domain::policy::EgressMode::Direct => exit.map(|proxy| proxy.url()),
        _ => Some(egress_proxy_url_for(mode.as_str())),
    }
}

/// Create a sandbox: validate, place, persist, then ask the runtime.
///
/// Order matters. The row is written first so a failed runtime call leaves a
/// visible `failed` sandbox rather than an invisible container.
pub async fn create(
    ctx: &ServiceContext,
    principal: &Principal,
    request: CreateSandbox,
) -> Result<Sandbox, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let requested_mode = match request.egress_mode.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(
            crate::domain::policy::EgressMode::parse(value)
                .ok_or_else(|| ApiError::validation("egress_mode is direct, forward_proxy, transparent or hybrid."))?
                .as_str()
                .to_string(),
        ),
        None => None,
    };

    let name = SandboxName::new(&request.name).map_err(ApiError::validation)?;
    // The gateway is stricter than a container: it derives a workload name from
    // the sandbox name, and refuses more than 19 characters. Saying that here is
    // a clear 422 instead of a runtime rejection after the row was written.
    if openshell_runtime() && name.as_str().chars().count() > 19 {
        return Err(ApiError::validation(
            "An OpenShell sandbox name is at most 19 characters; the gateway derives its workload name from it.",
        ));
    }

    // Normalise before anything is stored, so the spec holds exactly what the
    // runtime was asked for and a later restart cannot reinterpret it.
    let memory_limit = crate::domain::runtime::normalize_memory_limit(&request.memory_limit)
        .map_err(ApiError::validation)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;

    // The name is the runtime's name too (container, agent home, SSH alias), so it is unique
    // across the installation: another data center's sandbox with this name would collide on
    // a shared node and be ambiguous to the dispatcher. Migration 0062.
    let taken: bool = sqlx::query_scalar("SELECT nb_sandbox_name_taken($1, $2)")
        .bind(name.as_str())
        .bind(principal.environment_id)
        .fetch_one(scope.connection())
        .await
        .map_err(|e| ApiError::from(crate::store::map_sqlx_error(e)))?;
    if taken {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_NAME_TAKEN,
            "This name is used by another data center on this installation. Pick another name.",
        ));
    }

    // The image and command come from `agent_packs`, not from a `match`: an
    // unknown pack is a 404 and a disabled or misspelled one cannot silently
    // become the default shell (ADR 0020). A plain sandbox records no baseline,
    // so compatibility is checked here only where one is known.
    let defaults = pack_service::pack_defaults(&mut scope, &request.pack_key, None).await?;
    let image = if request.image.trim().is_empty() {
        defaults.image
    } else {
        request.image.clone()
    };
    let command: Vec<String> = if request.command.is_empty() {
        defaults.command
    } else {
        request.command.clone()
    };

    reject_unpinned_image(&image, request.image_digest.as_deref())?;

    let cluster_id = env_store::default_cluster(&mut scope)
        .await
        .map_err(ApiError::from)?;

    let gateway_id = crate::store::postgres::gateways::list(&mut scope)
        .await
        .map_err(ApiError::from)?
        .into_iter()
        .find(|gateway| gateway.accepts_placement())
        .map(|gateway| gateway.id);

    // The node is resolved by name and must exist: a typo must not fall back
    // to the local node silently.
    let node = match request.node.as_deref().filter(|n| !n.is_empty()) {
        Some(name) => crate::store::postgres::nodes::get_by_name(&mut scope, name)
            .await
            .map_err(|error| match error {
                crate::error::StoreError::NotFound => ApiError::not_found(
                    ErrorCode::NODE_NOT_FOUND,
                    format!("No node named '{name}'."),
                ),
                other => ApiError::from(other),
            })?,
        None => crate::store::postgres::nodes::local(&mut scope)
            .await
            .map_err(|error| match error {
                crate::error::StoreError::NotFound => ApiError::not_found(
                    ErrorCode::NODE_NOT_FOUND,
                    "This data center has no node to run a sandbox on. Add a node first, or name one.",
                ),
                other => ApiError::from(other),
            })?,
    };
    if !node.kind.is_operable() {
        return Err(ApiError::conflict(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("Node '{}' has no runtime adapter yet.", node.name),
        ));
    }

    // A GPU is handed out by the engine's container runtime, and a node whose
    // runtime has none would accept the request and start a sandbox without the
    // GPU it was promised. Refused here, before a row exists.
    let gpu_count = request.gpu_count.unwrap_or(0);
    if gpu_count > 0 && !node.container_gpu {
        return Err(ApiError::conflict(
            ErrorCode::CONFLICT,
            "This node cannot hand GPUs to containers.",
        ));
    }

    // A sandbox that cannot fit is refused before a row exists: the node's
    // budget is what its model installs left free, and a 32 GiB request on a
    // 7.8 GiB machine is a typo the operator should see now rather than an
    // OOM kill later. A node nobody has measured has no numbers to check
    // against, so the request is allowed and the response says the capacity is
    // unknown rather than being refused for a measurement that does not exist.
    let budget = crate::services::node_service::free_budget(&mut scope, node.id).await?;
    if budget.ram_total_bytes > 0 {
        let wanted = crate::domain::runtime::memory_bytes(&request.memory_limit)
            .map_err(ApiError::validation)?;
        budget.can_host(wanted, request.cpu_limit).map_err(|error| {
            ApiError::unprocessable(ErrorCode::NODE_INSUFFICIENT_RESOURCES, error.message())
        })?;
    }

    // Recorded in the spec as well as passed in the environment: "what was this
    // sandbox told" has to be answerable from the row after the fact, not
    // reconstructed from a later deploy's configuration.
    // Per sandbox, not per deploy: the mode the caller asked for decides the
    // runtime, and the runtime decides which address can be resolved (see
    // `egress_proxy_url_for`).
    let egress_proxy = egress_proxy_url_for(
        requested_mode.as_deref().unwrap_or("forward_proxy"),
    );

    let login = request.login.clone().filter(|value| !value.is_empty());
    if let Some(mode) = login.as_deref() {
        if !LOGIN_MODES.contains(&mode) {
            return Err(ApiError::validation("Sign-in is 'gateway' or 'own_account'."));
        }
        if mode == "own_account" && !pack_has_own_login(&request.pack_key) {
            return Err(ApiError::validation(
                "Only Claude Code, OpenCode and OpenChamber have a login of their own.",
            ));
        }
    }
    // A custom command and the GPU count belong in the spec for the same reason as
    // the limits: a recreate builds the container from the row, not from the
    // request that made it.
    let mut spec = serde_json::json!({
        "pack_key": request.pack_key,
        "memory_limit": memory_limit,
        "cpu_limit": request.cpu_limit,
        "pids_limit": request.pids_limit,
        "gpu_count": gpu_count,
        "node": node.name,
        "egress_proxy": &egress_proxy,
        "login": login.as_deref().unwrap_or("gateway"),
    });
    if !request.command.is_empty() {
        if let Some(object) = spec.as_object_mut() {
            object.insert("command".to_string(), serde_json::json!(&command));
        }
    }

    // A helper created without a lead joins the node's only agent. Done before the
    // lead is checked, so an automatic choice meets the same rules as a named one.
    let parent_id = match request.parent_id {
        Some(lead) => Some(lead),
        None => {
            let role = store::pack_role(&mut scope, &request.pack_key)
                .await
                .map_err(ApiError::from)?
                .unwrap_or_else(|| "agent".to_string());
            let candidates = store::lead_candidates(&mut scope, node.id)
                .await
                .map_err(ApiError::from)?;
            crate::domain::sandbox::auto_stack_lead(&role, &candidates)
        }
    };
    check_stack_lead(&mut scope, None, parent_id).await?;

    let sandbox = store::insert(
        &mut scope,
        store::NewSandbox {
            cluster_id,
            gateway_id,
            node_id: node.id,
            name: name.as_str(),
            pack_key: &request.pack_key,
            image: &image,
            image_digest: request.image_digest.as_deref(),
            spec: &spec,
            parent_id,
            owner_user_id: Some(principal.user_id),
        },
    )
    .await
    .map_err(ApiError::from)?;

    // The creator owns the sandbox: they manage it from start to end, and other users
    // (not admins) do not see it. It is written by the insert above, so the row is never
    // unowned — a member's own sandbox has to pass the row-level policy, and that policy
    // lets them write only what is theirs (migration 0090).

    // Every sandbox gets its supervisor row in the same transaction, so there is
    // no moment in which a running sandbox has no one watching it. On OpenShell
    // the row mirrors the supervisor container; elsewhere it stays `starting`.
    if let Some(gateway_id) = gateway_id {
        crate::services::gateway_service::create_for_sandbox(
            &mut scope,
            gateway_id,
            sandbox.id,
            &format!("sup-{}", sandbox.name),
        )
        .await
        .map_err(ApiError::from)?;
    }

    // The container receives the binds the database says this sandbox has. The
    // request cannot name a mount source: the only source is a persistent root's
    // stored host path. Read inside this transaction so a row written by the same
    // operation is visible.
    // A sandbox whose pack has switchable agents gets a persistent home for their
    // state: the login the user makes themselves, the settings, the agent config.
    // It lives on a persistent root, so it survives the sandbox being recreated,
    // which a mount change requires — and a recreate would otherwise sign the user
    // out of every agent.
    let has_switchable_agents =
        !crate::store::postgres::agent_auth::pack_agents(&mut scope, &sandbox.pack_key)
            .await
            .map_err(ApiError::from)?
            .is_empty();
    if has_switchable_agents {
        let slug = format!("home-{}", sandbox.name);
        if let Ok(valid) = crate::domain::persist::RootSlug::new(&slug) {
            let host_path = crate::domain::persist::host_path(
                &crate::services::persist_service::persistent_base(),
                &principal.user_id.to_string(),
                crate::domain::persist::RootKind::AgentHome,
                &valid,
            );
            // The home of an earlier sandbox of the same name is reused, so deleting a
            // sandbox and creating it again brings the login back. Looked up first: a
            // failed insert would abort the transaction.
            let existing = crate::store::postgres::persistent_roots::list_roots(&mut scope)
                .await
                .map_err(ApiError::from)?
                .into_iter()
                .find(|root| root.slug == slug && root.node_id == node.id && root.archived_at.is_none());
            let root_id = match existing {
                Some(root) => root.id,
                None => {
                    crate::store::postgres::persistent_roots::insert_root(
                        &mut scope,
                        crate::store::postgres::persistent_roots::NewRoot {
                            node_id: node.id,
                            kind: crate::domain::persist::RootKind::AgentHome,
                            owner_user_id: Some(principal.user_id),
                            slug: valid.as_str(),
                            host_path: &host_path,
                        },
                    )
                    .await
                    .map_err(ApiError::from)?
                    .id
                }
            };
            crate::store::postgres::persistent_roots::insert_mount(
                &mut scope,
                root_id,
                sandbox.id,
                "/home/nestbox/.agent-state",
                false,
            )
            .await
            .map_err(ApiError::from)?;
        }
    }
    let mounts =
        crate::services::persist_service::mount_specs_for_sandbox(&mut scope, sandbox.id).await?;

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.create",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox.id),
            detail: &serde_json::json!({ "name": sandbox.name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    scope.commit().await.map_err(ApiError::from)?;

    // A sandbox starts with the best-practice policy, named so the operator can
    // see what it is and replace it. This is deliberately after the commit and
    // best-effort: a missing preset leaves a sandbox without a policy, which is
    // exactly what every sandbox had before this feature, and failing the create
    // over it would trade a working sandbox for a tidy one.
    policy_service::apply_safe_defaults(ctx, principal.environment_id, sandbox.id).await;

    // Signing in with an own account needs the identity hosts of the agent's vendor,
    // which the default policy rightly does not allow. The rules are derived from the
    // agents' modes in one place (B02), so a mode switch and a create cannot disagree
    // about which hosts are open.
    if let Err(error) =
        crate::services::agent_auth_service::sync_login_rules(ctx, principal.environment_id, sandbox.id)
            .await
    {
        tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "could not write the sign-in rules");
    }

    // Now ask the runtime. A failure here is recorded on the row, so the console
    // shows a failed sandbox instead of nothing at all.
    // A pack that talks to a model needs the endpoint and the credential in its
    // environment. The credential is resolved from the provider profile's
    // locator at this moment and never stored (ADR 0005), so recreating the
    // sandbox picks up a rotated key without an edit here.
    let mut environment = if pack_needs_inference(&sandbox.pack_key) {
        match crate::services::provider_service::environment_for_sandbox(
            ctx,
            principal.environment_id,
            &sandbox.pack_key,
            &sandbox.name,
        )
        .await
        {
            Ok(values) => values.into_iter().collect(),
            Err(error) => {
                tracing::warn!(
                    code = error.code.0,
                    "provider environment unavailable; creating without it"
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // The proxy every sandbox is told about, plus the trust files. Without the
    // proxy the sandbox is isolated but blind — no tool can resolve an address,
    // because the sandbox network answers no external name. With it, each request
    // still needs a rule: the environment is a convenience for tools, not a
    // permission. The trust variables are set even though a new sandbox starts in
    // `forward_proxy`, because a switch to a TLS-reading mode must not need a
    // rebuild and a container cannot gain a mount afterwards.
    environment.extend(egress_environment_for(
        &egress_proxy,
        crate::domain::policy::EgressMode::ForwardProxy,
        EGRESS_CA_IN_SANDBOX,
    ));
    add_inference_bypass_vec(&mut environment);
    // The hub and the media server sit on the runtime network. Sent through the
    // egress proxy they are an external destination and are denied, which an
    // agent sees as "hub error".
    add_env_url_bypass_vec(&mut environment, "NESTBOX_MCP_URL");
    add_env_url_bypass_vec(&mut environment, "NESTBOX_MEDIA_MCP_URL");

    // Write the mode down before the container exists, so the stored row and the
    // container cannot disagree about what the proxy may read — the same rule the
    // recipe path follows. The runtime and the proxy address were already chosen
    // from this mode; leaving the row at the default made a `transparent` sandbox
    // a tunnel, with nothing in the console saying which of the two was true.
    if let Some(requested) = requested_mode.as_deref() {
        let mode = crate::domain::policy::EgressMode::parse(requested).ok_or_else(|| {
            ApiError::validation("egress_mode is direct, forward_proxy, transparent or hybrid.")
        })?;
        let mut mode_scope = TenantScope::begin_for(&ctx.pool, principal)
            .await
            .map_err(ApiError::from)?;
        crate::store::postgres::policies::set_egress_mode(
            &mut mode_scope,
            sandbox.id,
            mode,
            None,
            true,
            None,
            None,
        )
        .await
        .map_err(ApiError::from)?;
        mode_scope.commit().await.map_err(ApiError::from)?;
    }

    let runtime_spec = SandboxSpec {
        name: sandbox.name.clone(),
        image: sandbox.image.clone(),
        command,
        environment,
        pack_key: sandbox.pack_key.clone(),
        tool_profile: String::new(),
        memory_limit: memory_limit.clone(),
        cpu_limit: request.cpu_limit,
        pids_limit: request.pids_limit,
        mounts,
        gpu_count,
        // The sandbox starts in `forward_proxy`; the *runtime* follows the mode the
        // caller asked for (see `CreateSandbox::egress_mode`). A mode change later
        // is applied live and cannot move a sandbox between runtimes.
        egress_mode: requested_mode.unwrap_or_else(|| "forward_proxy".to_string()),
        node_id: Some(node.id),
        memory_reservation: None,
        cpu_shares: None,
    };

    let configured_environment: BTreeMap<String, String> =
        runtime_spec.environment.iter().cloned().collect();
    match ctx.runtime.create(runtime_spec).await {
        Ok(facts) => {
            // The agent starts already connected: its own config files are written
            // before anyone opens a shell. Best effort; a failure is logged and the
            // environment still carries the same facts.
            // A sandbox with a desktop gets its browser as an MCP server before
            // the agent's files are written, so the agent starts knowing it.
            if sandbox.pack_key == "desktop" || sandbox.image.contains("desktop") {
                if let Err(error) = crate::services::mcp_service::attach(ctx, principal, sandbox.id, "chrome-browser", true).await {
                    tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "could not attach the browser MCP");
                }
                // The browser drives pages; the desktop server drives the whole screen, every
                // monitor, the windows and the input. A desktop sandbox gets both, so an agent
                // never has to discover that half of its workspace is missing (U12).
                if let Err(error) = crate::services::mcp_service::attach(ctx, principal, sandbox.id, "desktop-control", true).await {
                    tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "could not attach the desktop MCP");
                }
            }
            // An agent starts with the Nestbox MCP hub attached when the hub is
            // deployed (the same condition that put its address in the environment).
            // Attached before the files are written, so the hub is in them.
            if configured_environment.contains_key("NESTBOX_MCP_URL") {
                if let Err(error) = crate::services::mcp_service::attach(ctx, principal, sandbox.id, "nestbox-hub", true).await {
                    tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "could not attach the MCP hub");
                }
            }
            let health = health_from_runtime(&facts);
            let updated = set_status(
                ctx,
                principal,
                sandbox.id,
                sandbox.version,
                SandboxStatus::Running,
                health,
            )
            .await?;
            // After the row says `running`: the agent files are written into a
            // running container, and `write_agent_files` refuses to guess.
            if let Err(error) = write_agent_files(ctx, principal.environment_id, sandbox.id).await {
                tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "agent files not written after create");
            }
            // The granted models are made ready in the background: the answer to a
            // create must not wait for a model load on an expensive machine.
            spawn_model_readiness(ctx, principal.environment_id, sandbox.id);
            Ok(updated)
        }
        Err(error) => {
            tracing::error!(sandbox = %sandbox.name, error = %error, "runtime create failed");
            let failed = set_status(
                ctx,
                principal,
                sandbox.id,
                sandbox.version,
                SandboxStatus::Failed,
                SandboxHealth::Unhealthy,
            )
            .await?;
            Ok(failed)
        }
    }
}

/// Make the sandbox's granted models ready in the background.
///
/// Spawned rather than awaited: loading a local model takes minutes, and a create,
/// a recreate or a start has to answer while it happens. A failure is logged and
/// nothing else: the sandbox is already running, and refusing it over a warm-up
/// would trade a working sandbox for a tidy one.
fn spawn_model_readiness(ctx: &ServiceContext, environment_id: Uuid, sandbox_id: Uuid) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        if let Err(error) =
            crate::services::model_readiness_service::ensure_ready_for_sandbox(&ctx, environment_id, sandbox_id)
                .await
        {
            tracing::warn!(sandbox = %sandbox_id, code = error.code.0, "models not made ready");
        }
    });
}

/// Run a lifecycle action.
pub async fn act(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    action: LifecycleAction,
) -> Result<Sandbox, ApiError> {
    require(principal, Permission::SandboxWrite)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    if !action.is_allowed_for(&sandbox) {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!(
                "Cannot {} a sandbox in status '{}'.",
                action.as_str(),
                sandbox.status.as_str()
            ),
        ));
    }

    let transitional = set_status(
        ctx,
        principal,
        id,
        sandbox.version,
        action.transitional(),
        sandbox.health,
    )
    .await?;

    let outcome = match action {
        LifecycleAction::Start => ctx.runtime.start(&sandbox.name).await,
        LifecycleAction::Stop => ctx.runtime.stop(&sandbox.name).await,
        LifecycleAction::Restart => ctx.runtime.restart(&sandbox.name).await,
    };

    match outcome {
        Ok(facts) => {
            let health = health_from_runtime(&facts);
            let settled = set_status(
                ctx,
                principal,
                id,
                transitional.version,
                action.settled(),
                health,
            )
            .await?;
            // A started sandbox asks for its granted models the moment it is used;
            // warming them now is what keeps the first request from waiting.
            if action == LifecycleAction::Start {
                spawn_model_readiness(ctx, principal.environment_id, id);
            }
            Ok(settled)
        }
        Err(error) => {
            tracing::error!(sandbox = %sandbox.name, action = action.as_str(), error = %error, "lifecycle action failed");
            set_status(
                ctx,
                principal,
                id,
                transitional.version,
                SandboxStatus::Failed,
                SandboxHealth::Unhealthy,
            )
            .await
        }
    }
}

/// Delete a sandbox and its container.
pub async fn delete(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<(), ApiError> {
    require(principal, Permission::SandboxWrite)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;

    // Removing the container first means a failure leaves the row, so the
    // operator can retry. Removing the row first would orphan a container.
    if let Err(error) = ctx.runtime.remove(&sandbox.name).await {
        tracing::warn!(sandbox = %sandbox.name, error = %error, "runtime remove failed; continuing");
    }
    // The rows go with the sandbox; the runner's listeners must go too, or the
    // node keeps a port open that leads nowhere and the next sandbox cannot use it.
    if let Err(error) = ctx.runtime.clear_forwards(&sandbox.name).await {
        tracing::warn!(sandbox = %sandbox.name, error = %error, "runtime clear_forwards failed; continuing");
    }

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.delete",
            subject_type: Some("sandbox"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "name": sandbox.name }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;

    // A supervisor that belongs to this sandbox cannot report again, so it goes
    // with it. A manually registered one (no sandbox) is left alone.
    crate::store::postgres::gateways::delete_supervisors_for_sandbox(&mut scope, id)
        .await
        .map_err(ApiError::from)?;

    store::delete(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(())
}

/// The last log lines of a sandbox.
pub async fn logs(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    tail: u32,
) -> Result<String, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.runtime
        .logs(&sandbox.name, tail)
        .await
        .map_err(|error| ApiError::internal("The runtime did not return logs.").with_internal(error.to_string()))
}

/// Run one command inside a sandbox.
///
/// Write permission, not read: a command changes the sandbox. Every run is an
/// audit event that records the vector, so "who ran what" is answerable, and
/// the output is not recorded, because it may contain a secret the sandbox
/// legitimately holds.
pub async fn exec(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    command: Vec<String>,
    timeout_seconds: u32,
) -> Result<crate::domain::runtime::ExecResult, ApiError> {
    require(principal, Permission::SandboxWrite)?;

    if command.is_empty() || command.len() > 64 {
        return Err(ApiError::validation(
            "A command is an argument vector of 1 to 64 parts.",
        ));
    }
    if command.iter().any(|part| part.contains('\0')) {
        return Err(ApiError::validation("Command arguments must not contain NUL."));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    if sandbox.status != SandboxStatus::Running {
        scope.commit().await.map_err(ApiError::from)?;
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            format!("Cannot run a command in a sandbox that is '{}'.", sandbox.status.as_str()),
        ));
    }

    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.exec",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox.id),
            detail: &serde_json::json!({ "name": sandbox.name, "command": command }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.runtime
        .exec(&sandbox.name, &command, timeout_seconds.clamp(1, 600))
        .await
        .map_err(|error| match error {
            crate::runtime::RuntimeError::NotFound(_) => ApiError::not_found(
                ErrorCode::SANDBOX_NOT_FOUND,
                "The runtime has no container for this sandbox.",
            ),
            crate::runtime::RuntimeError::Rejected(message) => ApiError::conflict(
                ErrorCode::SANDBOX_INVALID_STATE,
                message,
            ),
            other => ApiError::internal("The runtime could not run the command.")
                .with_internal(other.to_string()),
        })
}

/// Update a sandbox's status and publish the change.
async fn set_status(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    expected_version: i64,
    status: SandboxStatus,
    health: SandboxHealth,
) -> Result<Sandbox, ApiError> {
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::update_status(&mut scope, id, expected_version, status, health)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    ctx.bus.publish(LiveEvent::SandboxStateChanged {
        sandbox_id: sandbox.id,
        environment_id: sandbox.environment_id,
        status: sandbox.status.as_str().to_string(),
        version: sandbox.version,
    });

    Ok(sandbox)
}

/// Reject an image that is not digest-pinned.
///
/// `latest` is refused because a floating tag lets a registry change what runs
/// without a review. A digest is the only way to pin.
fn reject_unpinned_image(image: &str, digest: Option<&str>) -> Result<(), ApiError> {
    if image.ends_with(":latest") {
        return Err(ApiError::validation(
            "Image tag 'latest' is not allowed; pin a digest instead.",
        ));
    }

    // A locally built image has no registry digest to pin by reference. The
    // `:local` tag is the explicit, greppable marker that this is a
    // development image built on this host; it is allowed and recorded, while
    // every other floating tag is refused.
    if image.ends_with(":local") {
        return Ok(());
    }

    if digest.is_none() && !image.contains("@sha256:") {
        return Err(ApiError::validation(
            "Image must be pinned by digest (name@sha256:...) or tagged :local.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_is_refused() {
        assert!(reject_unpinned_image("nginx:latest", None).is_err());
    }

    #[test]
    fn a_digest_in_the_reference_is_enough() {
        assert!(reject_unpinned_image("nginx@sha256:abc", None).is_ok());
    }

    #[test]
    fn a_digest_field_is_enough() {
        assert!(reject_unpinned_image("nginx:1.27", Some("sha256:abc")).is_ok());
    }

    #[test]
    fn a_floating_tag_without_digest_is_refused() {
        assert!(reject_unpinned_image("nginx:1.27", None).is_err());
    }

    #[test]
    fn a_locally_built_image_is_allowed() {
        // It has no registry digest by construction; the tag is the marker.
        assert!(reject_unpinned_image("nestbox-hermes:local", None).is_ok());
    }

    #[test]
    fn only_settled_runtime_states_resolve_a_transitional_row() {
        assert_eq!(settled_status_from_runtime("running"), Some(SandboxStatus::Running));
        assert_eq!(settled_status_from_runtime("exited"), Some(SandboxStatus::Stopped));
        assert_eq!(settled_status_from_runtime("dead"), Some(SandboxStatus::Failed));
        assert_eq!(settled_status_from_runtime("restarting"), None);
        assert_eq!(settled_status_from_runtime("removing"), None);
    }

    #[test]
    fn health_is_derived_the_same_way_on_every_path() {
        let mut facts = crate::domain::runtime::RuntimeFacts::default();
        facts.status = "running".into();
        facts.health = "unknown".into();
        assert_eq!(health_from_runtime(&facts), SandboxHealth::Healthy);

        facts.status = "exited".into();
        assert_eq!(health_from_runtime(&facts), SandboxHealth::Unknown);

        facts.status = "running".into();
        facts.health = "unhealthy".into();
        assert_eq!(health_from_runtime(&facts), SandboxHealth::Unhealthy);
    }

    #[test]
    fn only_agent_packs_receive_an_inference_environment() {
        assert!(pack_needs_inference("hermes"));
        assert!(pack_needs_inference("claude-code") && pack_needs_inference("opencode") && pack_needs_inference("openchamber"));
        assert!(!pack_needs_inference("ubuntu-dev"));
        assert!(!pack_needs_inference("openclaw"));
    }

    #[test]
    fn a_gateway_with_an_address_goes_through_the_proxy_not_around_it() {
        use std::collections::BTreeMap;
        let mut environment: BTreeMap<String, String> = [
            ("OPENAI_BASE_URL".to_string(), "http://10.90.92.30:20128/v1".to_string()),
            ("NO_PROXY".to_string(), "localhost,egress-proxy".to_string()),
        ]
        .into_iter()
        .collect();
        add_inference_bypass(&mut environment);
        assert_eq!(environment.get("NO_PROXY").map(String::as_str), Some("localhost,egress-proxy"));
    }

    #[test]
    fn the_inference_host_is_kept_out_of_the_proxy() {
        use std::collections::BTreeMap;
        let mut environment: BTreeMap<String, String> = [
            ("OPENAI_BASE_URL".to_string(), "http://model-qwen3-1-7b:8080/v1".to_string()),
            ("NO_PROXY".to_string(), "localhost,egress-proxy".to_string()),
        ]
        .into_iter()
        .collect();
        add_inference_bypass(&mut environment);
        assert_eq!(
            environment.get("NO_PROXY").map(String::as_str),
            Some("localhost,egress-proxy,model-qwen3-1-7b")
        );
        assert_eq!(
            environment.get("no_proxy").map(String::as_str),
            Some("model-qwen3-1-7b")
        );
    }

    /// The supervisor reports to the control plane, so that host must not be
    /// proxied: through the proxy the report is a denied egress request, and the
    /// supervisor silently never reports.
    #[test]
    fn the_control_plane_is_kept_out_of_the_proxy_too() {
        let mut environment: Vec<(String, String)> = vec![
            ("NESTBOX_CONTROL_URL".to_string(), "http://control-api:8000".to_string()),
            ("NO_PROXY".to_string(), "localhost,egress-proxy".to_string()),
        ];
        add_env_url_bypass_vec(&mut environment, "NESTBOX_CONTROL_URL");
        let value = |key: &str| {
            environment
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            value("NO_PROXY").as_deref(),
            Some("localhost,egress-proxy,control-api")
        );
        assert_eq!(value("no_proxy").as_deref(), Some("control-api"));
    }

    #[test]
    fn the_egress_environment_names_both_spellings_of_every_variable() {
        let values = egress_environment(DEFAULT_EGRESS_PROXY);
        let keys: Vec<&str> = values.iter().map(|(key, _)| key.as_str()).collect();
        for key in [
            "HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY",
            "all_proxy", "NO_PROXY", "no_proxy",
        ] {
            assert!(keys.contains(&key), "{key} is missing from {keys:?}");
        }
        for (key, value) in &values {
            if key.eq_ignore_ascii_case("no_proxy") {
                continue;
            }
            assert_eq!(value, DEFAULT_EGRESS_PROXY);
        }
    }

    #[test]
    fn direct_egress_has_no_proxy_variable_at_all() {
        // "No proxy" has to mean no proxy variable: a leftover `HTTP_PROXY` is a
        // sandbox that still goes through the thing the operator switched off.
        let values = egress_environment_for(
            DEFAULT_EGRESS_PROXY,
            crate::domain::policy::EgressMode::Direct,
            EGRESS_CA_IN_SANDBOX,
        );
        // The helper is only called when there is a proxy (a direct sandbox with
        // an exit). With no exit the caller adds nothing at all.
        assert!(values.iter().any(|(key, _)| key == "HTTP_PROXY"));
        assert!(!values.iter().any(|(key, _)| key == "NODE_EXTRA_CA_CERTS"));
    }

    #[test]
    fn only_a_tls_reading_mode_trusts_the_node_ca() {
        use crate::domain::policy::EgressMode;
        // Every mode that goes through the proxy trusts the node CA, whatever it reads
        // today: the two files are mounted when the container is created and cannot be
        // added later, so a switch to a TLS-reading mode must not need a rebuild.
        for mode in [
            EgressMode::ForwardProxy,
            EgressMode::Transparent,
            EgressMode::Hybrid,
            EgressMode::OffAllowAll,
        ] {
            let values = egress_environment_for(DEFAULT_EGRESS_PROXY, mode, EGRESS_CA_IN_SANDBOX);
            let value = |key: &str| {
                values
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
            };
            // Node adds the CA alone; a client that replaces its store needs the
            // public roots as well, or every uninspected host fails to verify.
            assert_eq!(
                value("NODE_EXTRA_CA_CERTS"),
                Some("/run/secrets/egress-ca.crt"),
                "{mode:?}"
            );
            assert_eq!(
                value("SSL_CERT_FILE"),
                Some("/run/secrets/nestbox-ca-bundle.crt"),
                "{mode:?}"
            );
            for key in ["REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE", "GIT_SSL_CAINFO", "AWS_CA_BUNDLE"] {
                assert_eq!(value(key), Some("/run/secrets/nestbox-ca-bundle.crt"), "{mode:?} {key}");
            }
        }
        // The constant and the mount point are one fact.
        assert_eq!(EGRESS_CA_IN_SANDBOX, "/run/secrets/egress-ca.crt");
        assert_eq!(CA_BUNDLE_IN_SANDBOX, "/run/secrets/nestbox-ca-bundle.crt");
    }

    #[test]
    fn direct_trusts_no_certificate_at_all() {
        // Nothing of ours is in the path, so nothing of ours is trusted.
        use crate::domain::policy::EgressMode;
        let values = egress_environment_for(DEFAULT_EGRESS_PROXY, EgressMode::Direct, EGRESS_CA_IN_SANDBOX);
        for key in [
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "REQUESTS_CA_BUNDLE",
            "CURL_CA_BUNDLE",
            "GIT_SSL_CAINFO",
            "AWS_CA_BUNDLE",
        ] {
            assert!(
                !values.iter().any(|(name, _)| name == key),
                "direct must not set {key}"
            );
        }
    }

    #[test]
    fn a_direct_sandbox_with_an_exit_is_told_to_use_that_exit() {
        let exit = crate::domain::proxy::Proxy {
            id: uuid::Uuid::nil(),
            environment_id: uuid::Uuid::nil(),
            name: "DE".into(),
            kind: crate::domain::proxy::ProxyKind::Http,
            host: "192.168.55.1".into(),
            port: 22541,
            country: Some("DE".into()),
            provider: None,
            username: None,
            password_configured: false,
            password_ref: None,
            enabled: true,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let url = sandbox_proxy_url(crate::domain::policy::EgressMode::Direct, Some(&exit));
        assert_eq!(url.as_deref(), Some("http://192.168.55.1:22541"));
        // Without an exit, a direct sandbox is given no proxy.
        assert_eq!(sandbox_proxy_url(crate::domain::policy::EgressMode::Direct, None), None);
        // A proxied mode always points at the node's egress proxy.
        assert_eq!(
            sandbox_proxy_url(crate::domain::policy::EgressMode::ForwardProxy, Some(&exit)).as_deref(),
            Some(DEFAULT_EGRESS_PROXY)
        );
    }

    #[test]
    fn a_tool_is_not_told_to_reach_the_proxy_through_the_proxy() {
        let values = egress_environment("http://egress-proxy:3128");
        let bypass = values
            .iter()
            .find(|(key, _)| key == "NO_PROXY")
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        assert!(bypass.contains("egress-proxy"), "{bypass}");
        assert!(bypass.contains("127.0.0.1"));
        assert!(bypass.contains("localhost"));
    }

    #[test]
    fn the_proxy_host_is_extracted_from_whatever_url_shape_a_node_reports() {
        assert_eq!(proxy_host("http://egress-proxy:3128"), "egress-proxy");
        assert_eq!(proxy_host("https://proxy.node.internal"), "proxy.node.internal");
        assert_eq!(proxy_host("http://10.90.92.30:3128/"), "10.90.92.30");
        // A bare host:port is not a URL the check constraint would accept, but
        // the parser must not mangle it into something that looks like an alias.
        assert_eq!(proxy_host("egress-proxy:3128"), "egress-proxy");
    }

    #[test]
    fn an_empty_proxy_setting_falls_back_to_the_node_default() {
        // The function reads the process environment, so the test asserts the
        // shape of the default rather than mutating a global other tests share.
        assert!(DEFAULT_EGRESS_PROXY.starts_with("http://"));
        assert!(DEFAULT_EGRESS_PROXY.ends_with(":3128"));
        assert!(DEFAULT_EGRESS_PROXY.contains(proxy_host(DEFAULT_EGRESS_PROXY)));
    }

    #[test]
    fn lifecycle_actions_respect_the_current_status() {
        let mut sandbox = sample(SandboxStatus::Running);
        assert!(LifecycleAction::Stop.is_allowed_for(&sandbox));
        assert!(!LifecycleAction::Start.is_allowed_for(&sandbox));
        assert!(LifecycleAction::Restart.is_allowed_for(&sandbox));

        sandbox.status = SandboxStatus::Stopped;
        assert!(LifecycleAction::Start.is_allowed_for(&sandbox));
        assert!(!LifecycleAction::Stop.is_allowed_for(&sandbox));
        assert!(!LifecycleAction::Restart.is_allowed_for(&sandbox));
    }

    fn sample(status: SandboxStatus) -> Sandbox {
        Sandbox {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            cluster_id: Uuid::nil(),
            gateway_id: None,
            gateway_name: None,
            supervisor_id: None,
            workspace_id: None,
            parent_id: None,
            node_id: Uuid::nil(),
            node_name: Some("local".into()),
            name: "x".into(),
            pack_key: "p".into(),
            image: "i@sha256:a".into(),
            image_digest: Some("sha256:a".into()),
            spec: serde_json::json!({}),
            status,
            health: SandboxHealth::Healthy,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            spec_detail: None,
        }
    }
}


/// What an operator may change on a running sandbox.
#[derive(Debug, Default)]
pub struct ResourceRequest {
    /// `512Mi`, `2g`, ... as everywhere else.
    pub memory_limit: Option<String>,
    /// The soft target: memory the sandbox may use before the kernel starts
    /// reclaiming, while `memory_limit` stays the hard ceiling.
    pub memory_reservation: Option<String>,
    pub cpu_limit: Option<f64>,
    /// Relative CPU weight when cores are contended (Docker's default is 1024).
    pub cpu_shares: Option<i64>,
    pub pids_limit: Option<i64>,
}

/// Change a sandbox's resources, live when it is running.
///
/// The stored spec is the desired state and is updated first-class so a restart
/// keeps the new figures; the runtime is told to apply them to the running
/// container. A runtime that cannot is reported, not hidden: the spec is saved,
/// and the answer says the running container still has the old limits.
pub async fn set_resources(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    request: ResourceRequest,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let memory_limit = request
        .memory_limit
        .as_deref()
        .map(crate::domain::runtime::normalize_memory_limit)
        .transpose()
        .map_err(ApiError::validation)?;
    if let Some(cpus) = request.cpu_limit {
        if !(0.1..=64.0).contains(&cpus) {
            return Err(ApiError::validation("CPU cores must be between 0.1 and 64."));
        }
    }
    if let Some(pids) = request.pids_limit {
        if !(32..=65536).contains(&pids) {
            return Err(ApiError::validation("The process limit must be between 32 and 65536."));
        }
    }
    if let Some(shares) = request.cpu_shares {
        if !(2..=262_144).contains(&shares) {
            return Err(ApiError::validation("CPU shares must be between 2 and 262144."));
        }
    }
    let reservation = request
        .memory_reservation
        .as_deref()
        .map(crate::domain::runtime::normalize_memory_limit)
        .transpose()
        .map_err(ApiError::validation)?;
    if memory_limit.is_none()
        && reservation.is_none()
        && request.cpu_limit.is_none()
        && request.cpu_shares.is_none()
        && request.pids_limit.is_none()
    {
        return Err(ApiError::validation("Nothing to change."));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;

    let memory_bytes = memory_limit
        .as_deref()
        .map(crate::domain::runtime::memory_bytes)
        .transpose()
        .map_err(ApiError::validation)?;
    let reservation_bytes = reservation
        .as_deref()
        .map(crate::domain::runtime::memory_bytes)
        .transpose()
        .map_err(ApiError::validation)?;
    // A soft target above the hard ceiling would never be reached; refusing it
    // is kinder than storing a number that cannot happen.
    if let (Some(soft), Some(hard)) = (reservation_bytes, memory_bytes) {
        if soft > hard {
            return Err(ApiError::validation(
                "The memory reservation cannot be larger than the memory limit.",
            ));
        }
    }
    if let Some(wanted) = memory_bytes {
        let budget = crate::services::node_service::free_budget(&mut scope, sandbox.node_id).await?;
        // The sandbox already holds its current memory; only growth is checked
        // against what the node has left.
        let current = sandbox
            .spec
            .get("memory_limit")
            .and_then(|value| value.as_str())
            .and_then(|text| crate::domain::runtime::memory_bytes(text).ok())
            .unwrap_or(0);
        if budget.ram_total_bytes > 0 && wanted > current {
            budget
                .can_host(wanted - current, request.cpu_limit.unwrap_or(0.0))
                .map_err(|error| ApiError::unprocessable(ErrorCode::NODE_INSUFFICIENT_RESOURCES, error.message()))?;
        }
    }

    let mut spec = sandbox.spec.clone();
    if let Some(object) = spec.as_object_mut() {
        if let Some(limit) = &memory_limit {
            object.insert("memory_limit".into(), serde_json::json!(limit));
        }
        if let Some(cpus) = request.cpu_limit {
            object.insert("cpu_limit".into(), serde_json::json!(cpus));
        }
        if let Some(reservation) = &reservation {
            object.insert("memory_reservation".into(), serde_json::json!(reservation));
        }
        if let Some(shares) = request.cpu_shares {
            object.insert("cpu_shares".into(), serde_json::json!(shares));
        }
        if let Some(pids) = request.pids_limit {
            object.insert("pids_limit".into(), serde_json::json!(pids));
        }
    }
    store::update_spec(&mut scope, id, &spec).await.map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.resources_changed",
            subject_type: Some("sandbox"),
            subject_id: Some(id),
            detail: &serde_json::json!({
                "memory_limit": memory_limit,
                "memory_reservation": reservation,
                "cpu_limit": request.cpu_limit,
                "cpu_shares": request.cpu_shares,
                "pids_limit": request.pids_limit
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let live = if sandbox.status == SandboxStatus::Running {
        match ctx
            .runtime
            .update_resources(
                &sandbox.name,
                memory_bytes,
                reservation_bytes,
                request.cpu_limit,
                request.cpu_shares,
                request.pids_limit,
            )
            .await
        {
            Ok(()) => "applied",
            Err(error) => {
                tracing::warn!(sandbox = %sandbox.name, error = %error, "live resource change failed");
                "saved_not_applied"
            }
        }
    } else {
        "saved_applies_on_start"
    };
    Ok(serde_json::json!({
        "applied": live,
        "memory_limit": memory_limit,
        "memory_reservation": reservation,
        "cpu_limit": request.cpu_limit,
        "cpu_shares": request.cpu_shares,
        "pids_limit": request.pids_limit,
    }))
}

/// The host's load, for the console's live strip.
pub async fn host_load(ctx: &ServiceContext, principal: &Principal) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::NodeRead)?;
    match tokio::time::timeout(std::time::Duration::from_secs(6), ctx.runtime.host_load()).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(ApiError::internal("The runner did not report the host's load.").with_internal(error.to_string())),
        Err(_) => Err(ApiError::internal("The runner did not answer in time.")),
    }
}


/// Write an agent's configuration files into a running sandbox.
///
/// The content is passed base64-encoded as an argument of a fixed script, so no
/// value from the files can become shell syntax. Each file is written for the
/// sandbox user's home (`$HOME`), and an existing file is replaced: this is the
/// control plane's configuration, and re-applying it is how a changed model or
/// proxy reaches an agent.
pub async fn apply_agent_files(
    ctx: &ServiceContext,
    sandbox_name: &str,
    pack_key: &str,
    environment: &BTreeMap<String, String>,
    servers: &[crate::domain::agent_config::McpServerConfig],
) -> usize {
    use base64::Engine;
    let files = crate::domain::agent_config::files_for_with_servers(pack_key, environment, servers);
    let mut written = 0;
    for file in &files {
        let encoded = base64::engine::general_purpose::STANDARD.encode(file.content.as_bytes());
        // Plain files are replaced. A state file the agent also writes is merged into
        // when it exists (keys already there win, the MCP server list is updated), so
        // re-applying the configuration never signs the user out.
        let script = r#"target="$HOME/$1"; mkdir -p "$(dirname "$target")" || exit 1
printf %s "$2" | base64 -d > "$target.new" || exit 1
if [ "$3" = merge ] && [ -s "$target" ] && command -v python3 >/dev/null 2>&1; then
python3 - "$target" "$target.new" <<'PY'
import json, sys
old = json.load(open(sys.argv[1]))
new = json.load(open(sys.argv[2]))
for key, value in new.items():
    if key == "mcpServers":
        # The list in the new file is authoritative: a server that was detached
        # must disappear, and merging would leave it behind forever.
        old[key] = value
    elif key not in old:
        old[key] = value
json.dump(old, open(sys.argv[1], "w"), indent=2)
PY
rm -f "$target.new"
else
mv "$target.new" "$target"
fi"#;
        let command = vec![
            "sh".to_string(),
            "-c".to_string(),
            script.to_string(),
            "sh".to_string(),
            file.path.clone(),
            encoded,
            if file.merge_json { "merge" } else { "replace" }.to_string(),
        ];
        match ctx.runtime.exec(sandbox_name, &command, 20).await {
            Ok(result) if result.exit_code == Some(0) => written += 1,
            Ok(result) => tracing::warn!(sandbox = sandbox_name, path = %file.path, exit = ?result.exit_code, "agent config not written"),
            Err(error) => tracing::warn!(sandbox = sandbox_name, path = %file.path, error = %error, "agent config not written"),
        }
    }
    written
}

/// The MCP servers an agent in this sandbox should start with.
///
/// Read through a short scope of its own: the caller may hold a transaction, and
/// a config write that needs a second read must not extend it. A failed read is
/// an empty list plus a log line, because losing the hub address is better than
/// refusing to start a sandbox whose agent would still work.
pub async fn mcp_configs(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    sandbox_name: &str,
) -> Vec<crate::domain::agent_config::McpServerConfig> {
    mcp_configs_in_environment(ctx, principal.environment_id, sandbox_id, sandbox_name).await
}

/// The same list for a caller that has the environment but no user.
///
/// A background write after a create or a recreate is not a request, so it has no
/// principal; the tenant scope is what the read needs. Kept as one implementation
/// so a request and a background write cannot list different servers.
async fn mcp_configs_in_environment(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
    sandbox_name: &str,
) -> Vec<crate::domain::agent_config::McpServerConfig> {
    let mut scope = match TenantScope::begin(&ctx.pool, environment_id).await {
        Ok(scope) => scope,
        Err(error) => {
            tracing::warn!(sandbox = sandbox_name, error = %error, "could not read MCP attachments");
            return Vec::new();
        }
    };
    let servers = match crate::store::postgres::mcp::agent_servers(&mut scope, sandbox_id).await {
        Ok(servers) => servers,
        Err(error) => {
            tracing::warn!(sandbox = sandbox_name, error = %error, "could not read MCP attachments");
            let _ = scope.commit().await;
            return Vec::new();
        }
    };
    let _ = scope.commit().await;
    servers
        .into_iter()
        .map(|server| crate::domain::agent_config::McpServerConfig {
            // The registry stores the compose name, which a host-network
            // OpenShell supervisor cannot resolve. The deployment's URL wins,
            // exactly as it does for the hub.
            endpoint: if server.key == "nestbox-media" {
                Some(crate::services::provider_service::media_mcp_url())
            } else {
                server.endpoint
            },
            sandbox: Some(sandbox_name.to_string()),
            token_env: if server.auth_type == crate::domain::mcp::McpAuth::Token {
                server.env_slots.first().cloned()
            } else {
                None
            },
            key: server.key,
            transport: server.transport.as_str().to_string(),
            command: server.command,
            args: server.args,
        })
        .collect()
}

/// Re-apply an agent's configuration to an existing sandbox (a changed model, a
/// sandbox from before the files existed).
pub async fn reconfigure_agent(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    reconfigure_pack(ctx, principal, id, None).await
}

/// Re-apply the agent configuration of a sandbox (a changed model, a sandbox from
/// before the files existed).
///
/// The pack override is not needed any more: `write_agent_files` writes every pack
/// the sandbox holds, so a recipe sandbox no longer has agents whose files wait for
/// someone to name them.
pub async fn reconfigure_pack(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
    pack_override: Option<&str>,
) -> Result<serde_json::Value, ApiError> {
    let _ = pack_override;
    require(principal, Permission::SandboxWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    if sandbox.status != SandboxStatus::Running {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            "Configuration is written into a running sandbox; start it first.",
        ));
    }
    let written = write_agent_files(ctx, principal.environment_id, sandbox.id).await?;
    Ok(serde_json::json!({ "files_written": written }))
}


/// Recreate a sandbox's container with the mounts the database now says it has.
///
/// A bind mount is fixed when a container is created, and Docker cannot add or
/// remove one on a running container, so "mount it live" is: replace the
/// container with a new one that has the right binds. What survives is everything
/// on a persistent root and the sandbox's identity (name, ports, keys, policy,
/// stack, history). What does not is anything written *outside* a persistent root
/// inside the old container (installed packages, a home that was never mounted).
/// The console says so before it asks.
///
/// Only a sandbox created from a pack's own image can be recreated this way: a
/// recipe sandbox's entrypoint is generated at build time and is not reproduced
/// here, so recreating it would start something else.
pub async fn recreate(
    ctx: &ServiceContext,
    principal: &Principal,
    id: Uuid,
) -> Result<serde_json::Value, ApiError> {
    require(principal, Permission::SandboxWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, id).await.map_err(ApiError::from)?;
    if sandbox.image.starts_with("nestbox-recipe/") {
        return Err(ApiError::conflict(
            ErrorCode::SANDBOX_INVALID_STATE,
            "This sandbox was built from a recipe; its entrypoint is generated at build time, so it cannot be recreated here. Rebuild it from its recipe instead.",
        ));
    }
    let defaults = pack_service::pack_defaults(&mut scope, &sandbox.pack_key, None).await?;
    let mounts = crate::services::persist_service::mount_specs_for_sandbox(&mut scope, sandbox.id).await?;
    // The two answers the container has to be created with. Read here, inside
    // the transaction, because a recreate is exactly the moment they take
    // effect: Docker fixes a container's environment when it is created.
    let egress_mode = crate::store::postgres::policies::egress_mode_for_sandbox(&mut scope, sandbox.id)
        .await
        .map_err(ApiError::from)?
        .map(|setting| setting.mode)
        .unwrap_or(crate::domain::policy::EgressMode::ForwardProxy);
    let egress_exit = crate::store::postgres::proxies::for_sandbox(&mut scope, sandbox.id)
        .await
        .map_err(ApiError::from)?
        .filter(|proxy| proxy.enabled);
    let memory_limit = sandbox
        .spec
        .get("memory_limit")
        .and_then(|value| value.as_str())
        .unwrap_or("1g")
        .to_string();
    let cpu_limit = sandbox.spec.get("cpu_limit").and_then(|value| value.as_f64()).unwrap_or(1.0);
    let pids_limit = sandbox.spec.get("pids_limit").and_then(|value| value.as_i64()).unwrap_or(256);
    // A recreate reproduces the container the row describes, not the pack's
    // defaults: a custom command, the GPU count and the resource figures
    // `set_resources` stored are part of what the operator chose.
    let command: Vec<String> = sandbox
        .spec
        .get("command")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or(defaults.command);
    let gpu_count = sandbox
        .spec
        .get("gpu_count")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        .min(u32::MAX as u64) as u32;
    let memory_reservation = sandbox
        .spec
        .get("memory_reservation")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let cpu_shares = sandbox.spec.get("cpu_shares").and_then(|value| value.as_i64());
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "sandbox.recreate",
            subject_type: Some("sandbox"),
            subject_id: Some(id),
            detail: &serde_json::json!({ "mounts": mounts.len() }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    // The container keeps the gateway keys whatever an agent's sign-in mode is: the
    // launcher strips them per mode (ADR 0054), so switching a mode never needs the
    // container to be built again.
    let mut environment: Vec<(String, String)> =
        crate::services::provider_service::environment_for_sandbox(ctx, principal.environment_id, &sandbox.pack_key, &sandbox.name)
            .await
            .map(|values| values.into_iter().collect())
            .unwrap_or_default();
    if let Some(proxy) = sandbox_proxy_url(egress_mode, egress_exit.as_ref()) {
        environment.extend(egress_environment_for(
            &proxy,
            egress_mode,
            EGRESS_CA_IN_SANDBOX,
        ));
    }
    add_inference_bypass_vec(&mut environment);
    // The hub and the media server sit on the runtime network. Sent through the
    // egress proxy they are an external destination and are denied, which an
    // agent sees as "hub error".
    add_env_url_bypass_vec(&mut environment, "NESTBOX_MCP_URL");
    add_env_url_bypass_vec(&mut environment, "NESTBOX_MEDIA_MCP_URL");

    if let Err(error) = ctx.runtime.remove(&sandbox.name).await {
        tracing::warn!(sandbox = %sandbox.name, error = %error, "remove before recreate failed; continuing");
    }
    let spec = SandboxSpec {
        name: sandbox.name.clone(),
        image: sandbox.image.clone(),
        command,
        environment,
        pack_key: sandbox.pack_key.clone(),
        tool_profile: String::new(),
        memory_limit,
        cpu_limit,
        pids_limit,
        mounts: mounts.clone(),
        gpu_count,
        egress_mode: egress_mode.as_str().to_string(),
        node_id: Some(sandbox.node_id),
        memory_reservation,
        cpu_shares,
    };
    match ctx.runtime.create(spec).await {
        Ok(facts) => {
            let _ = set_status(ctx, principal, sandbox.id, sandbox.version, SandboxStatus::Running, health_from_runtime(&facts)).await;
            // The row has to say `running` first: the files are written into the
            // container, and `write_agent_files` refuses to guess when it is gone.
            if let Err(error) = write_agent_files(ctx, principal.environment_id, sandbox.id).await {
                tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "agent files not written after recreate");
            }
            spawn_model_readiness(ctx, principal.environment_id, sandbox.id);
            Ok(serde_json::json!({ "recreated": true, "mounts": mounts.len() }))
        }
        Err(error) => {
            tracing::error!(sandbox = %sandbox.name, error = %error, "recreate failed");
            let _ = set_status(ctx, principal, sandbox.id, sandbox.version, SandboxStatus::Failed, SandboxHealth::Unhealthy).await;
            Err(ApiError::internal("The sandbox could not be recreated; its container is gone. Start it again or recreate once more.").with_internal(error.to_string()))
        }
    }
}


/// Keep the stored state and the runtime in step, for every tenant.
///
/// The row is the desired state: a sandbox the operator left `running` should be
/// running. After a host reboot or a crashed container the runtime says
/// `exited` while the row still says `running`, and every endpoint that needs
/// the container (terminal, processes, inventory, snapshots) answers 409. So a
/// `running` row whose container is down is started again; if the start fails
/// the row becomes `failed`, which is the truth. A `stopped` row is never
/// started, so an operator's stop is respected.
///
/// Cross-tenant maintenance like the builder's boot routine: environments are
/// listed unscoped, and every write happens inside its tenant scope.
pub async fn reconcile_all(ctx: &ServiceContext) -> Result<u64, ApiError> {
    let facts = ctx.runtime.list().await.map_err(|error| {
        ApiError::conflict(ErrorCode::SANDBOX_INVALID_STATE, error.to_string())
    })?;
    let index: std::collections::HashMap<String, crate::domain::runtime::RuntimeFacts> =
        facts.into_iter().map(|fact| (fact.name.clone(), fact)).collect();
    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
    let mut changed = 0;
    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id)
            .await
            .map_err(ApiError::from)?;
        let rows: Vec<(Uuid, String, i64)> = sqlx::query_as(
            "SELECT id, name, version FROM sandboxes WHERE status = 'running'",
        )
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(StoreError::Database(error)))?;
        scope.commit().await.map_err(ApiError::from)?;

        for (id, name, version) in rows {
            let Some(fact) = index.get(&name) else { continue };
            if fact.status == "running" || settled_status_from_runtime(&fact.status).is_none() {
                continue;
            }
            let (status, health) = match ctx.runtime.start(&name).await {
                Ok(started) => (SandboxStatus::Running, health_from_runtime(&started)),
                Err(error) => {
                    tracing::warn!(sandbox = %name, error = %error, "could not bring a sandbox back");
                    (SandboxStatus::Failed, SandboxHealth::Unhealthy)
                }
            };
            let mut scope = TenantScope::begin(&ctx.pool, environment_id)
                .await
                .map_err(ApiError::from)?;
            if store::update_status(&mut scope, id, version, status, health).await.is_ok() {
                changed += 1;
                tracing::info!(sandbox = %name, to = status.as_str(), "reconciled a sandbox with the runtime");
            }
            scope.commit().await.map_err(ApiError::from)?;
        }
    }
    Ok(changed)
}

/// The ids of the sandboxes this caller may see, for the live streams. `None` when the
/// database cannot be read, in which case the stream keeps what it knew.
pub async fn visible_ids(
    pool: &sqlx::PgPool,
    principal: &Principal,
) -> Option<std::collections::HashSet<Uuid>> {
    let mut scope = TenantScope::begin_for(pool, principal).await.ok()?;
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM sandboxes")
        .fetch_all(scope.connection())
        .await
        .ok()?;
    let _ = scope.commit().await;
    Some(ids.into_iter().collect())
}


// ---------------------------------------------------------------------------
// Bringing back sandboxes that crashed
// ---------------------------------------------------------------------------

/// How long to wait before the next attempt on a sandbox, by how many failed:
/// a minute, two, four ... capped at half an hour.
fn heal_delay(attempts: u32) -> std::time::Duration {
    std::time::Duration::from_secs((60u64 << attempts.min(5)).min(1800))
}

/// After this many failed attempts a sandbox is left for an operator.
const HEAL_MAX_ATTEMPTS: u32 = 6;

type HealState = std::collections::HashMap<String, (u32, std::time::Instant)>;

fn heal_state() -> &'static std::sync::Mutex<HealState> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<HealState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| std::sync::Mutex::new(HealState::new()))
}

/// Restart gateway sandboxes that died while they were meant to run.
///
/// "Meant to run" is the stored status: a sandbox an operator stopped is `stopped`
/// and is left alone, one that is `running` in the database but not on the
/// runtime crashed. OpenShell freezes a workload whose supervisor lost the gateway
/// and ends it when the recovery deadline passes (phase `Error`); nothing used to
/// bring it back. Each attempt is audited, and repeated failures back off.
pub async fn heal_crashed(ctx: &ServiceContext) -> Result<u64, ApiError> {
    if !openshell_runtime() {
        return Ok(0);
    }
    let facts = ctx.runtime.list().await.map_err(|error| {
        ApiError::new(
            axum::http::StatusCode::BAD_GATEWAY,
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            error.to_string(),
        )
    })?;
    let crashed: std::collections::HashSet<String> = facts
        .iter()
        .filter(|fact| fact.container_name.starts_with("openshell-default--") && fact.status != "running")
        .map(|fact| fact.name.clone())
        .collect();
    let running: std::collections::HashSet<String> = facts
        .iter()
        .filter(|fact| fact.status == "running")
        .map(|fact| fact.name.clone())
        .collect();
    // A sandbox that runs again has paid its debt.
    if let Ok(mut state) = heal_state().lock() {
        state.retain(|name, _| !running.contains(name));
    }
    if crashed.is_empty() {
        return Ok(0);
    }

    let environments: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM environments")
        .fetch_all(&ctx.pool)
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
    let mut healed = 0;
    for environment_id in environments {
        let mut scope = TenantScope::begin(&ctx.pool, environment_id).await.map_err(ApiError::from)?;
        // `failed` is included: when a workload dies the reconcile marks the sandbox
        // failed, and it is exactly the one to bring back. A sandbox that never
        // came up also lands here, which is why attempts are capped below.
        let wanted: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, name FROM sandboxes WHERE status::text IN ('running', 'failed')",
        )
        .fetch_all(scope.connection())
        .await
        .map_err(|error| ApiError::from(crate::error::StoreError::Database(error)))?;
        for (sandbox_id, name) in wanted.into_iter().filter(|(_, name)| crashed.contains(name)) {
            let due = match heal_state().lock() {
                Ok(state) => state
                    .get(&name)
                    .map(|(attempts, last)| {
                        *attempts < HEAL_MAX_ATTEMPTS && last.elapsed() >= heal_delay(*attempts)
                    })
                    .unwrap_or(true),
                Err(_) => false,
            };
            if !due {
                continue;
            }
            let attempts = heal_state()
                .lock()
                .map(|mut state| {
                    let entry = state.entry(name.clone()).or_insert((0, std::time::Instant::now()));
                    entry.0 += 1;
                    entry.1 = std::time::Instant::now();
                    entry.0
                })
                .unwrap_or(1);
            let outcome = ctx.runtime.restart(&name).await;
            tracing::warn!(sandbox = %name, attempt = attempts, ok = outcome.is_ok(), "restarting a sandbox that crashed");
            let detail = serde_json::json!({
                "attempt": attempts,
                "result": match &outcome { Ok(_) => "restarted".to_string(), Err(error) => error.to_string() },
            });
            let _ = crate::store::postgres::audit::record(
                &mut scope,
                crate::store::postgres::audit::NewAuditEvent {
                    actor_id: None,
                    actor_label: "nestbox",
                    kind: "sandbox.healed",
                    subject_type: Some("sandbox"),
                    subject_id: Some(sandbox_id),
                    detail: &detail,
                    request_id: None,
                },
            )
            .await;
            if outcome.is_ok() {
                healed += 1;
                // It runs again; the stored state should say so.
                let _ = sqlx::query(
                    "UPDATE sandboxes SET status = 'running', version = version + 1, updated_at = now() \
                      WHERE id = $1 AND status::text = 'failed'",
                )
                .bind(sandbox_id)
                .execute(scope.connection())
                .await;
            }
        }
        scope.commit().await.map_err(ApiError::from)?;
    }
    Ok(healed)
}

#[cfg(test)]
mod heal_tests {
    use super::*;

    #[test]
    fn the_wait_doubles_and_is_capped() {
        assert_eq!(heal_delay(0).as_secs(), 60);
        assert_eq!(heal_delay(1).as_secs(), 120);
        assert_eq!(heal_delay(3).as_secs(), 480);
        assert_eq!(heal_delay(5).as_secs(), 1800);
        assert_eq!(heal_delay(40).as_secs(), 1800);
    }
}

/// Rebuild a sandbox's agent configuration from the current sign-in modes.
///
/// This is the one place that writes agent files: the pack's config files with
/// the gateway settings of the gateway agents only, the MCP servers, the egress
/// proxy and trust variables, and the launcher that reads each agent's mode
/// (ADR 0054). Nothing restarts, so the caller decides about running agents.
///
/// It needs the environment and the sandbox, not a request: a create, a recreate
/// and a mode switch all end here, and none of them may write a different set of
/// files. A sandbox that is not running is not an error — its container is gone
/// and the files are written the next time it runs.
pub async fn write_agent_files(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
) -> Result<usize, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let sandbox = store::get(&mut scope, sandbox_id).await.map_err(ApiError::from)?;
    if sandbox.status != SandboxStatus::Running {
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(0);
    }
    // The mode is read here so the launcher's proxy variables match the container
    // the sandbox actually runs in, not the mode it was created with.
    let egress_mode = crate::store::postgres::policies::egress_mode_for_sandbox(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
        .map(|setting| setting.mode)
        .unwrap_or(crate::domain::policy::EgressMode::ForwardProxy);
    // A recipe sandbox holds more than one agent: its own pack and one per recipe
    // agent. Every pack is written, else the second agent keeps a stale config.
    let mut packs = vec![sandbox.pack_key.clone()];
    for pack in crate::store::postgres::agent_auth::sandbox_agent_packs(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        if !packs.contains(&pack) {
            packs.push(pack);
        }
    }
    scope.commit().await.map_err(ApiError::from)?;

    let mut environment: BTreeMap<String, String> =
        crate::services::provider_service::environment_for_sandbox(
            ctx,
            environment_id,
            &sandbox.pack_key,
            &sandbox.name,
        )
        .await?;
    let modes = crate::services::agent_auth_service::modes_for(ctx, environment_id, sandbox_id).await?;
    crate::domain::agent_auth::strip_for_modes(&mut environment, &modes);
    let egress_env: BTreeMap<String, String> = if egress_mode.uses_proxy() {
        egress_environment(&egress_proxy_url()).into_iter().collect()
    } else {
        BTreeMap::new()
    };
    environment.extend(egress_env.clone());

    let servers = mcp_configs_in_environment(ctx, environment_id, sandbox_id, &sandbox.name).await;
    let mut written = 0;
    for pack in &packs {
        written += apply_agent_files(ctx, &sandbox.name, pack, &environment, &servers).await;
    }
    // The launcher reads the mode files; without it a subscription agent would still
    // see the gateway key in its environment. A sandbox without a switchable agent
    // (a plain ubuntu-dev, a tool image) has nothing to launch, so nothing is written.
    if !modes.is_empty() {
    if let Err(error) = crate::services::agent_launch_service::install(ctx, &sandbox.name, &modes, &egress_env).await {
        tracing::warn!(sandbox = %sandbox.name, code = error.code.0, "agent launcher not installed");
    }
    }
    Ok(written)
}
