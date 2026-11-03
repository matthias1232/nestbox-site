//! The stack diagram: one sandbox stack, from the operator to the internet.
//!
//! An operator opening a sandbox asks "what is this working with, and how is it
//! wired?". The answer is a graph in nine layers, drawn top to bottom:
//!
//! | layer | what is in it |
//! |---|---|
//! | 0 `operator` | you: a terminal, a browser, an agent client |
//! | 1 `ports` | the ports published on the node, each with its dial address |
//! | 2 `sandboxes` | the stack's lead and its members, the agent inside each |
//! | 3 `tools` | MCP servers attached to the sandboxes |
//! | 4 `supervisor` | the process that watches a sandbox and applies policy |
//! | 5 `control` | the control API, the OpenShell gateway(s), the runner |
//! | 6 `egress` | the proxy every outbound request goes through |
//! | 7 `destinations` | hosts the stack actually talked to, with blocked ones marked |
//! | 8 `internet` | everything beyond the proxy |
//!
//! This module only *arranges* facts the service has already read. It invents
//! none: a layer with nothing in it says so with an `absent` node (for a
//! supervisor that is not bound, for example) instead of being left out, because
//! "there is no supervisor here" is exactly what an operator needs to see.
//!
//! Every node carries a `status` from one small vocabulary so the console colours
//! by meaning and not by guessing at strings:
//!
//! * `ok` — works and answers;
//! * `warn` — works, but something is off (stale, drifted, open with nothing behind it);
//! * `error` — failed or unreachable;
//! * `idle` — deliberately not running (stopped);
//! * `unknown` — nobody could be asked;
//! * `absent` — the thing does not exist for this stack.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::runtime::ComponentState;

/// A supervisor that has not reported for this long is called stale.
pub const SUPERVISOR_STALE_SECONDS: i64 = 120;

/// The layers, top to bottom. The console draws them in this order.
pub const LAYERS: &[(&str, &str)] = &[
    ("operator", "You"),
    ("ports", "Ports on the node"),
    ("sandboxes", "Sandbox stack"),
    ("tools", "Tools and MCP"),
    ("supervisor", "Supervisor"),
    ("control", "Control plane and gateway"),
    ("egress", "Egress proxy (Docker sandboxes)"),
    ("destinations", "Destinations"),
    ("internet", "Internet"),
];

fn layer_index(key: &str) -> usize {
    LAYERS.iter().position(|(name, _)| *name == key).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Facts: what the service read
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HostFacts {
    pub name: String,
    pub address: Option<String>,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SandboxFacts {
    pub id: Uuid,
    pub name: String,
    pub pack_key: String,
    /// `agent`, `tool` or `gateway`.
    pub role: String,
    pub summary: Option<String>,
    pub status: String,
    pub health: String,
    pub parent_id: Option<Uuid>,
    /// The address on the sandbox network. Not reachable from outside; shown
    /// because it is what the other pieces of the stack call this sandbox.
    pub internal_address: Option<String>,
    pub gateway_name: Option<String>,
    /// Enforced by an NVIDIA OpenShell supervisor, which mediates every connection
    /// itself; such a sandbox does not go through the egress proxy.
    pub openshell: bool,
}

#[derive(Debug, Clone)]
pub struct PortFacts {
    pub sandbox_id: Uuid,
    pub forward_id: Uuid,
    pub purpose: String,
    pub label: String,
    /// `ssh`, `web`, `api` or `tcp`.
    pub kind: String,
    pub description: Option<String>,
    pub target_port: i32,
    pub public_port: i32,
    pub dial: Option<String>,
    pub url: Option<String>,
    pub listening: Option<bool>,
    pub target_up: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct McpFacts {
    pub sandbox_id: Uuid,
    pub key: String,
    pub name: String,
    pub transport: String,
    pub endpoint: Option<String>,
    pub status: String,
    pub tools: usize,
    pub enabled: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SupervisorFacts {
    pub id: Uuid,
    pub name: String,
    pub sandbox_id: Option<Uuid>,
    pub status: String,
    pub version: Option<String>,
    pub config_dirtiness: i32,
    pub last_seen: Option<DateTime<Utc>>,
    pub checks_failing: usize,
    pub checks_warning: usize,
}

#[derive(Debug, Clone)]
pub struct GatewayFacts {
    pub id: Uuid,
    pub name: String,
    pub status: String,
    pub drifted: bool,
    pub endpoint: String,
    pub used: i32,
    pub capacity: i32,
}

#[derive(Debug, Clone)]
pub struct EgressFacts {
    pub sandbox_id: Uuid,
    pub host: String,
    pub port: i32,
    pub protocol: String,
    pub requests: i64,
    pub allowed: i64,
    pub denied: i64,
}

/// One name the stack asked for and how the policy answered.
///
/// A refused DNS lookup leaves no network event: the request was never made, so
/// the traffic rows cannot show it. This is the evidence that the sandbox needed
/// a name and could not resolve it, which is a different failure from a blocked
/// connection and needs its own card.
#[derive(Debug, Clone)]
pub struct DnsFacts {
    pub host: String,
    /// `allow`, `deny` or `audit`.
    pub decision: String,
    pub count: i64,
}

/// Everything the builder needs. Plain data, so a test builds one in a few lines.
pub struct StackInput {
    pub subject: Uuid,
    pub now: DateTime<Utc>,
    pub window: String,
    pub host: HostFacts,
    /// The lead first, then its members. A standalone sandbox is a stack of one.
    pub sandboxes: Vec<SandboxFacts>,
    pub ports: Vec<PortFacts>,
    pub mcp: Vec<McpFacts>,
    pub supervisors: Vec<SupervisorFacts>,
    pub gateways: Vec<GatewayFacts>,
    /// `None` when the runner could not be asked.
    pub components: Option<Vec<ComponentState>>,
    pub egress: Vec<EgressFacts>,
    /// DNS lookups the stack made in the same window as `egress`.
    pub dns: Vec<DnsFacts>,
}

// ---------------------------------------------------------------------------
// The answer
// ---------------------------------------------------------------------------

/// Where to read a node's log, in terms the console maps to an endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LogSource {
    /// `sandbox` (`GET /sandboxes/{id}/logs`) or `component`
    /// (`GET /system/components/{id}/logs`).
    pub kind: String,
    pub id: String,
    /// Keep only lines containing this text. A supervisor shares its sandbox's
    /// output with the agent, so its viewer reads the same log filtered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// A line of detail on a node: `label: value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Fact {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StackNode {
    pub id: String,
    /// `operator`, `port`, `sandbox`, `mcp`, `supervisor`, `control_api`,
    /// `gateway`, `runner`, `egress_proxy`, `component`, `destination`,
    /// `dns_blocked`, `internet`, `absent`.
    pub kind: String,
    pub layer: usize,
    pub layer_key: String,
    pub name: String,
    /// One sentence: what this is, in the operator's words.
    pub description: String,
    /// `ok`, `warn`, `error`, `idle`, `unknown`, `absent`.
    pub status: String,
    /// The raw state behind `status`, e.g. `running` or `connected`.
    pub state: String,
    pub facts: Vec<Fact>,
    /// Something the operator can open in a browser, or copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<LogSource>,
    /// The sandbox this node belongs to, so the console can focus on one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StackEdge {
    pub from: String,
    pub to: String,
    /// `ingress`, `forward`, `stack`, `mcp`, `supervises`, `reports`, `hosts`,
    /// `runs`, `publishes`, `egress`, `decides`, `reaches`.
    pub kind: String,
    pub label: String,
    /// Same vocabulary as a node's status.
    pub status: String,
    pub requests: i64,
    pub denied: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StackLayer {
    pub index: usize,
    pub key: String,
    pub label: String,
}

/// One problem with how urgent it is.
///
/// `severity` is a rank, not a word: 0 is the worst, 5 the most informational,
/// the same order `problems` is sorted in. The console colours by this number
/// instead of matching words in the sentence, which would break the moment the
/// wording changed and would mis-colour a sentence that merely mentions "block".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RankedProblem {
    pub text: String,
    /// 0 worst … 5 informational.
    pub severity: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct StackTopology {
    /// The sandbox the operator opened.
    pub subject: Uuid,
    /// The lead of the stack it belongs to (itself, when standalone).
    pub lead: Uuid,
    pub generated_at: DateTime<Utc>,
    pub window: String,
    pub layers: Vec<StackLayer>,
    pub nodes: Vec<StackNode>,
    pub edges: Vec<StackEdge>,
    /// Sentences about what is wrong or missing, worst first. Empty means
    /// nothing was found, not that nothing was checked.
    pub problems: Vec<String>,
    /// The same problems in the same order, each with its severity rank. Kept
    /// beside `problems` so a reader that only wants the sentences keeps the
    /// shape it had, and the console can colour without parsing the text.
    pub problems_ranked: Vec<RankedProblem>,
    /// How many nodes there are per status, for the header.
    pub counts: std::collections::BTreeMap<String, usize>,
}

// ---------------------------------------------------------------------------
// Status rules
// ---------------------------------------------------------------------------

/// A sandbox's status as the diagram colours it.
pub fn sandbox_status(status: &str, health: &str) -> &'static str {
    match status {
        "running" => match health {
            "unhealthy" => "error",
            "healthy" | "unknown" | "" => "ok",
            _ => "warn",
        },
        "starting" | "provisioning" | "pending" | "stopping" => "warn",
        "stopped" => "idle",
        "failed" => "error",
        _ => "unknown",
    }
}

/// A port's status: the runner's two facts, not the database's word.
pub fn port_status(listening: Option<bool>, target_up: Option<bool>) -> &'static str {
    match (listening, target_up) {
        (Some(true), Some(true)) => "ok",
        (Some(true), _) => "warn",
        (Some(false), _) => "error",
        (None, _) => "unknown",
    }
}

fn mcp_status(status: &str, enabled: bool) -> &'static str {
    if !enabled {
        return "idle";
    }
    match status {
        "connected" => "ok",
        "error" => "error",
        "disconnected" => "warn",
        _ => "unknown",
    }
}

fn supervisor_status(facts: &SupervisorFacts, now: DateTime<Utc>) -> &'static str {
    if facts.checks_failing > 0 {
        return "error";
    }
    let stale = facts
        .last_seen
        .map(|seen| now - seen > Duration::seconds(SUPERVISOR_STALE_SECONDS));
    match (facts.status.as_str(), stale) {
        (_, None) => "unknown",
        ("stopped" | "failed", _) => "error",
        (_, Some(true)) => "warn",
        ("starting" | "degraded" | "unknown", _) => "warn",
        _ if facts.checks_warning > 0 || facts.config_dirtiness > 0 => "warn",
        _ => "ok",
    }
}

fn component_status(component: &ComponentState) -> &'static str {
    match (component.state.as_str(), component.health.as_deref()) {
        ("running", Some("unhealthy")) => "error",
        ("running", Some("starting")) => "warn",
        ("running", _) => "ok",
        ("restarting", _) => "warn",
        ("missing", _) => "absent",
        ("exited" | "dead", _) => "error",
        _ => "unknown",
    }
}

fn gateway_status(facts: &GatewayFacts) -> &'static str {
    match facts.status.as_str() {
        "healthy" if facts.drifted => "warn",
        "healthy" => "ok",
        "degraded" => "warn",
        "unhealthy" | "offline" => "error",
        _ => "unknown",
    }
}

/// The worse of two statuses, so an edge or a rollup takes the more urgent one.
pub fn worse(a: &str, b: &str) -> String {
    fn rank(status: &str) -> u8 {
        match status {
            "error" => 5,
            "warn" => 4,
            "unknown" => 3,
            "idle" => 2,
            "absent" => 1,
            _ => 0,
        }
    }
    (if rank(b) > rank(a) { b } else { a }).to_string()
}

fn fact(label: &str, value: impl Into<String>) -> Fact {
    Fact { label: label.to_string(), value: value.into() }
}

fn node(
    id: String,
    kind: &str,
    layer_key: &str,
    name: impl Into<String>,
    description: impl Into<String>,
    status: &str,
    state: impl Into<String>,
) -> StackNode {
    StackNode {
        id,
        kind: kind.to_string(),
        layer: layer_index(layer_key),
        layer_key: layer_key.to_string(),
        name: name.into(),
        description: description.into(),
        status: status.to_string(),
        state: state.into(),
        facts: Vec::new(),
        url: None,
        logs: None,
        sandbox_id: None,
    }
}

fn edge(from: &str, to: &str, kind: &str, label: impl Into<String>, status: &str) -> StackEdge {
    StackEdge {
        from: from.to_string(),
        to: to.to_string(),
        kind: kind.to_string(),
        label: label.into(),
        status: status.to_string(),
        requests: 0,
        denied: 0,
    }
}

const OPERATOR: &str = "operator";
const INTERNET: &str = "internet";
const CONTROL_API: &str = "component:control-api";
const RUNNER: &str = "component:sandbox-runner";
const PROXY: &str = "component:egress-proxy";

fn sandbox_id_of(id: Uuid) -> String {
    format!("sandbox:{id}")
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// Arrange the facts into the diagram.
pub fn build(input: &StackInput) -> StackTopology {
    let mut nodes: Vec<StackNode> = Vec::new();
    let mut edges: Vec<StackEdge> = Vec::new();
    let mut problems: Vec<(u8, String)> = Vec::new();

    let lead = input
        .sandboxes
        .iter()
        .find(|sandbox| sandbox.parent_id.is_none())
        .map(|sandbox| sandbox.id)
        .unwrap_or(input.subject);

    // -- you ---------------------------------------------------------------------
    nodes.push(node(
        OPERATOR.into(),
        "operator",
        "operator",
        "You",
        "A terminal, a browser or an agent client on your machine.",
        "ok",
        "connected",
    ));

    // -- the sandboxes -----------------------------------------------------------
    for sandbox in &input.sandboxes {
        let status = sandbox_status(&sandbox.status, &sandbox.health);
        let is_lead = sandbox.id == lead && input.sandboxes.len() > 1;
        let mut item = node(
            sandbox_id_of(sandbox.id),
            "sandbox",
            "sandboxes",
            sandbox.name.clone(),
            sandbox.summary.clone().unwrap_or_else(|| {
                format!("A sandbox running the {} pack.", sandbox.pack_key)
            }),
            status,
            sandbox.status.clone(),
        );
        item.sandbox_id = Some(sandbox.id);
        item.facts.push(fact("Pack", sandbox.pack_key.clone()));
        item.facts.push(fact("Role", if is_lead { "lead".to_string() } else { sandbox.role.clone() }));
        item.facts.push(fact("Health", sandbox.health.clone()));
        if let Some(address) = &sandbox.internal_address {
            item.facts.push(fact("Sandbox network address", format!("{address} (inside the sandbox network only)")));
        }
        if let Some(gateway) = &sandbox.gateway_name {
            item.facts.push(fact("Placed on gateway", gateway.clone()));
        }
        item.logs = Some(LogSource { kind: "sandbox".into(), id: sandbox.id.to_string(), filter: None });
        nodes.push(item);

        if sandbox.id != lead {
            edges.push(edge(&sandbox_id_of(lead), &sandbox_id_of(sandbox.id), "stack", "stack member", status));
        }
        if status == "error" {
            problems.push((0, format!("Sandbox {} is {}.", sandbox.name, sandbox.status)));
        } else if status == "idle" {
            problems.push((3, format!("Sandbox {} is stopped.", sandbox.name)));
        }
    }

    // -- ports on the node -------------------------------------------------------
    let host_label = input
        .host
        .hostname
        .clone()
        .or_else(|| input.host.address.clone());
    for port in &input.ports {
        let status = port_status(port.listening, port.target_up);
        let id = format!("port:{}", port.forward_id);
        let mut item = node(
            id.clone(),
            "port",
            "ports",
            match &port.dial {
                Some(dial) => dial.clone(),
                None => format!(":{}", port.public_port),
            },
            port.description.clone().unwrap_or_else(|| {
                if port.purpose == "ssh" {
                    "SSH into the sandbox.".to_string()
                } else {
                    format!("Published port for container port {}.", port.target_port)
                }
            }),
            status,
            match (port.listening, port.target_up) {
                (Some(true), Some(true)) => "up",
                (Some(true), _) => "open, nothing behind it",
                (Some(false), _) => "closed",
                (None, _) => "unknown",
            },
        );
        item.sandbox_id = Some(port.sandbox_id);
        item.url = port.url.clone();
        item.facts.push(fact("For", port.label.clone()));
        item.facts.push(fact("Kind", port.kind.clone()));
        item.facts.push(fact("Node port", port.public_port.to_string()));
        item.facts.push(fact("Port in the sandbox", port.target_port.to_string()));
        if port.dial.is_none() {
            item.facts.push(fact("Dial", "the node has no address set"));
        }
        nodes.push(item);
        edges.push(edge(
            OPERATOR,
            &id,
            "ingress",
            match port.kind.as_str() {
                "ssh" => "ssh",
                "web" => "browser",
                "api" => "client",
                _ => "tcp",
            },
            status,
        ));
        edges.push(edge(
            &id,
            &sandbox_id_of(port.sandbox_id),
            "forward",
            format!("→ {}", port.target_port),
            status,
        ));
        if status == "error" {
            problems.push((1, format!("Port {} ({}) is not open on the node.", port.public_port, port.label)));
        } else if status == "warn" {
            problems.push((4, format!(
                "Port {} ({}) is open, but nothing answers inside the sandbox yet.",
                port.public_port, port.label
            )));
        }
    }
    if input.ports.iter().all(|port| port.kind != "ssh") && !input.ports.is_empty() {
        problems.push((2, "No SSH port is published for this stack.".to_string()));
    }

    // -- tools and MCP -----------------------------------------------------------
    for server in &input.mcp {
        let status = mcp_status(&server.status, server.enabled);
        let id = format!("mcp:{}:{}", server.sandbox_id, server.key);
        let mut item = node(
            id.clone(),
            "mcp",
            "tools",
            server.name.clone(),
            format!("An MCP server ({}) the agent can call as a tool.", server.transport),
            status,
            if !server.enabled {
                "disabled".to_string()
            } else if server.transport == "stdio" && server.status == "unknown" {
                // A stdio server is started by the agent inside the sandbox, so
                // nothing outside can probe it. "unknown" read like a fault.
                "runs in the sandbox".to_string()
            } else {
                server.status.clone()
            },
        );
        item.sandbox_id = Some(server.sandbox_id);
        item.facts.push(fact("Transport", server.transport.clone()));
        item.facts.push(fact("Tools", server.tools.to_string()));
        if let Some(endpoint) = &server.endpoint {
            item.facts.push(fact("Endpoint", endpoint.clone()));
        }
        if let Some(error) = &server.last_error {
            item.facts.push(fact("Last error", error.clone()));
        }
        nodes.push(item);
        edges.push(edge(&sandbox_id_of(server.sandbox_id), &id, "mcp", server.transport.clone(), status));
        if status == "error" {
            problems.push((1, format!("MCP server {} reports an error.", server.name)));
        }
    }

    // -- supervisors -------------------------------------------------------------
    for sandbox in &input.sandboxes {
        let watching: Vec<&SupervisorFacts> = input
            .supervisors
            .iter()
            .filter(|supervisor| supervisor.sandbox_id == Some(sandbox.id))
            .collect();
        if watching.is_empty() {
            // Only an agent needs watching; a helper tool without a supervisor
            // is ordinary, and a warning for every one of them would be noise.
            if sandbox.role == "agent" {
                let id = format!("absent:supervisor:{}", sandbox.id);
                let mut item = node(
                    id.clone(),
                    "absent",
                    "supervisor",
                    "No supervisor",
                    format!("Nothing watches {} or applies policy inside it.", sandbox.name),
                    "absent",
                    "none",
                );
                item.sandbox_id = Some(sandbox.id);
                nodes.push(item);
                edges.push(edge(&id, &sandbox_id_of(sandbox.id), "supervises", "not bound", "absent"));
            }
            continue;
        }
        for supervisor in watching {
            let status = supervisor_status(supervisor, input.now);
            let id = format!("supervisor:{}", supervisor.id);
            let mut item = node(
                id.clone(),
                "supervisor",
                "supervisor",
                supervisor.name.clone(),
                if sandbox.openshell {
                    format!(
                        "The trusted OpenShell supervisor of {}. It sits outside the sandbox, checks every connection against the policy, adds credentials only where policy allows, and freezes the agent if it loses the gateway.",
                        sandbox.name
                    )
                } else {
                    format!("Watches {} and applies the policy the control plane assigns.", sandbox.name)
                },
                status,
                supervisor.status.clone(),
            );
            item.sandbox_id = Some(sandbox.id);
            if let Some(version) = &supervisor.version {
                item.facts.push(fact("Version", version.clone()));
            }
            item.facts.push(fact(
                "Last report",
                supervisor
                    .last_seen
                    .map(|seen| format!("{}s ago", (input.now - seen).num_seconds().max(0)))
                    .unwrap_or_else(|| "never".to_string()),
            ));
            item.facts.push(fact("Pending configuration", supervisor.config_dirtiness.to_string()));
            if supervisor.checks_failing + supervisor.checks_warning > 0 {
                item.facts.push(fact(
                    "Checks",
                    format!("{} failing, {} warning", supervisor.checks_failing, supervisor.checks_warning),
                ));
            }
            // The supervisor prints to its sandbox's output, next to the agent.
            item.logs = Some(LogSource {
                kind: "sandbox".into(),
                id: sandbox.id.to_string(),
                filter: Some("nestbox-supervisor".into()),
            });
            nodes.push(item);
            edges.push(edge(&id, &sandbox_id_of(sandbox.id), "supervises", "watches", status));
            edges.push(edge(&id, CONTROL_API, "reports", "reports", status));
            match status {
                "error" => problems.push((0, format!("Supervisor {} reports a failing check or is down.", supervisor.name))),
                "warn" => problems.push((3, format!("Supervisor {} is stale or has pending configuration.", supervisor.name))),
                "unknown" => problems.push((4, format!("Supervisor {} has never reported.", supervisor.name))),
                _ => {}
            }
        }
    }

    // -- the control plane -------------------------------------------------------
    let component = |service: &str| -> Option<&ComponentState> {
        input.components.as_ref().and_then(|all| all.iter().find(|item| item.service == service))
    };
    let system_node = |id: &str, kind: &str, service: &str, label: &str, fallback: &str| -> StackNode {
        let found = component(service);
        let (status, state, description) = match (&input.components, found) {
            (_, Some(item)) => (component_status(item), item.state.clone(), item.description.clone()),
            (Some(_), None) => ("absent", "missing".to_string(), fallback.to_string()),
            (None, None) => ("unknown", "unknown".to_string(), fallback.to_string()),
        };
        let mut item = node(id.to_string(), kind, "control", label, description, status, state);
        if let Some(found) = found {
            if let Some(image) = &found.image {
                item.facts.push(fact("Image", image.clone()));
            }
            if let Some(started) = &found.started_at {
                item.facts.push(fact("Started", started.clone()));
            }
            if found.restarts > 0 {
                item.facts.push(fact("Restarts", found.restarts.to_string()));
            }
            if let Some(health) = &found.health {
                item.facts.push(fact("Health check", health.clone()));
            }
        }
        item.logs = Some(LogSource { kind: "component".into(), id: service.to_string(), filter: None });
        item
    };

    // The control API answered this request, so it is running by definition; the
    // runner's view adds its image and start time when it could be asked.
    let mut api = system_node(CONTROL_API, "control_api", "control-api", "Control API",
        "The control plane: API, policy decisions, audit.");
    if api.status != "ok" {
        api.status = "ok".to_string();
        api.state = "running".to_string();
    }
    nodes.push(api);
    let runner = system_node(RUNNER, "runner", "sandbox-runner", "Sandbox runner",
        "Starts sandboxes, opens their published ports and the SSH door.");
    let runner_status = runner.status.clone();
    nodes.push(runner);

    for gateway in &input.gateways {
        let status = gateway_status(gateway);
        let id = format!("gateway:{}", gateway.id);
        let mut item = node(
            id.clone(),
            "gateway",
            "control",
            gateway.name.clone(),
            "An OpenShell gateway that carries sandboxes.",
            status,
            gateway.status.clone(),
        );
        item.facts.push(fact("Endpoint", gateway.endpoint.clone()));
        item.facts.push(fact("Load", format!("{} of {} sandboxes", gateway.used, gateway.capacity)));
        if gateway.drifted {
            item.facts.push(fact("Configuration", "differs from what the control plane wants"));
            problems.push((3, format!("Gateway {} runs an older configuration than desired.", gateway.name)));
        }
        nodes.push(item);
        for sandbox in &input.sandboxes {
            if sandbox.gateway_name.as_deref() == Some(gateway.name.as_str()) {
                edges.push(edge(&id, &sandbox_id_of(sandbox.id), "hosts", "carries", status));
            }
        }
        if status == "error" {
            problems.push((0, format!("Gateway {} is not reachable.", gateway.name)));
        }
    }

    // The runner publishes the ports and runs the sandboxes.
    for port in &input.ports {
        edges.push(edge(RUNNER, &format!("port:{}", port.forward_id), "publishes", "listens", &runner_status));
    }
    for sandbox in &input.sandboxes {
        edges.push(edge(RUNNER, &sandbox_id_of(sandbox.id), "runs", "runs", &runner_status));
    }

    // -- the egress proxy, destinations, internet ---------------------------------
    // The proxy is drawn only when a sandbox of the stack goes through it. An
    // OpenShell sandbox does not: its supervisor mediates every connection.
    let via_proxy = input.sandboxes.iter().any(|sandbox| !sandbox.openshell);
    let proxy = system_node(PROXY, "egress_proxy", "egress-proxy", "Egress proxy",
        "The only way out of the sandbox network. Asks the control API per request; default deny.");
    let proxy_status = proxy.status.clone();
    if via_proxy {
        if proxy_status == "error" || proxy_status == "absent" {
            problems.push((0, "The egress proxy is not running: sandboxes have no way out.".to_string()));
        }
        nodes.push(proxy);
        edges.push(edge(PROXY, CONTROL_API, "decides", "asks per request", &proxy_status));
    }
    // Where a sandbox's connections leave from: its supervisor, or the proxy.
    let exit_of = |sandbox: &SandboxFacts| -> String {
        if sandbox.openshell {
            if let Some(supervisor) = input
                .supervisors
                .iter()
                .find(|supervisor| supervisor.sandbox_id == Some(sandbox.id))
            {
                return format!("supervisor:{}", supervisor.id);
            }
        }
        PROXY.to_string()
    };

    for sandbox in input.sandboxes.iter().filter(|sandbox| !sandbox.openshell) {
        let mut totals = (0i64, 0i64);
        for row in input.egress.iter().filter(|row| row.sandbox_id == sandbox.id) {
            totals.0 += row.requests;
            totals.1 += row.denied;
        }
        let mut link = edge(
            &sandbox_id_of(sandbox.id),
            PROXY,
            "egress",
            if totals.0 == 0 { "no traffic".to_string() } else { format!("{} requests", totals.0) },
            if totals.1 > 0 { "warn" } else { &proxy_status },
        );
        link.requests = totals.0;
        link.denied = totals.1;
        edges.push(link);
    }

    // One node per destination, one edge from the proxy; the sandboxes' own
    // edges to the proxy already say who asked.
    let mut destinations: Vec<(String, i32, String, i64, i64, i64)> = Vec::new();
    for row in &input.egress {
        match destinations
            .iter_mut()
            .find(|(host, port, ..)| host == &row.host && *port == row.port)
        {
            Some(entry) => {
                entry.3 += row.requests;
                entry.4 += row.allowed;
                entry.5 += row.denied;
            }
            None => destinations.push((row.host.clone(), row.port, row.protocol.clone(), row.requests, row.allowed, row.denied)),
        }
    }
    destinations.sort_by(|a, b| b.3.cmp(&a.3).then(a.0.cmp(&b.0)));
    const MAX_DESTINATIONS: usize = 12;
    let hidden = destinations.len().saturating_sub(MAX_DESTINATIONS);
    for (host, port, protocol, requests, allowed, denied) in destinations.into_iter().take(MAX_DESTINATIONS) {
        let id = format!("dest:{host}:{port}");
        let status = if denied > 0 { "warn" } else { "ok" };
        let mut item = node(
            id.clone(),
            "destination",
            "destinations",
            format!("{host}:{port}"),
            if denied > 0 {
                "A host the stack tried to reach. Some requests were blocked.".to_string()
            } else {
                "A host the stack reached through the proxy.".to_string()
            },
            status,
            if denied > 0 { "blocked in part" } else { "reached" },
        );
        item.facts.push(fact("Protocol", protocol));
        item.facts.push(fact("Requests", requests.to_string()));
        item.facts.push(fact("Allowed", allowed.to_string()));
        item.facts.push(fact("Blocked", denied.to_string()));
        nodes.push(item);
        // One edge from each exit that reached it: the proxy for Docker sandboxes,
        // the sandbox's own supervisor on OpenShell.
        let mut exits: Vec<(String, i64, i64)> = Vec::new();
        for row in input.egress.iter().filter(|row| row.host == host && row.port == port) {
            let Some(sandbox) = input.sandboxes.iter().find(|sandbox| sandbox.id == row.sandbox_id) else {
                continue;
            };
            let exit = exit_of(sandbox);
            match exits.iter_mut().find(|(name, ..)| *name == exit) {
                Some(entry) => {
                    entry.1 += row.requests;
                    entry.2 += row.denied;
                }
                None => exits.push((exit, row.requests, row.denied)),
            }
        }
        for (exit, exit_requests, exit_denied) in exits {
            let mut link = edge(&exit, &id, "reaches", format!("{exit_requests} req"), status);
            link.requests = exit_requests;
            link.denied = exit_denied;
            edges.push(link);
        }
        edges.push(edge(&id, INTERNET, "reaches", "", status));
        if denied > 0 {
            problems.push((2, format!("{denied} request(s) to {host}:{port} were blocked by policy.")));
        }
    }
    if hidden > 0 {
        problems.push((5, format!("{hidden} more destination(s) are not drawn; the Traffic tab lists all of them.")));
    }

    // A name the policy refused to resolve is its own kind of wall: no request is
    // ever made, so no destination card can show it. One card per host, because
    // the failure is the name, not the sandbox that happened to ask for it.
    let mut blocked_dns: Vec<(String, i64)> = Vec::new();
    for row in input.dns.iter().filter(|row| row.decision == "deny") {
        match blocked_dns.iter_mut().find(|(host, _)| host == &row.host) {
            Some(entry) => entry.1 += row.count,
            None => blocked_dns.push((row.host.clone(), row.count)),
        }
    }
    blocked_dns.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (host, count) in blocked_dns {
        let mut item = node(
            format!("dns:{host}"),
            "dns_blocked",
            "destinations",
            format!("{host} (DNS blocked)"),
            "A name this stack asked for. The policy refused to resolve it, so nothing could connect.",
            "error",
            "blocked",
        );
        item.facts.push(fact("Decision", "deny"));
        item.facts.push(fact("Blocked lookups", count.to_string()));
        nodes.push(item);
        problems.push((
            0,
            format!("DNS for {host} was blocked {count} time(s); the sandbox cannot resolve it."),
        ));
    }
    nodes.push(node(
        INTERNET.into(),
        "internet",
        "internet",
        "Internet",
        "Everything beyond the supervisor or proxy: model APIs, package registries, the web.",
        "ok",
        "beyond the boundary",
    ));
    if input.egress.is_empty() {
        let exit = input.sandboxes.first().map(&exit_of).unwrap_or_else(|| PROXY.to_string());
        edges.push(edge(&exit, INTERNET, "reaches", "no traffic in the window", "idle"));
    }

    // Keep the order stable: by layer, then insertion, so the console does not
    // shuffle cards between refreshes.
    nodes.sort_by_key(|item| item.layer);

    let mut counts = std::collections::BTreeMap::new();
    for item in &nodes {
        *counts.entry(item.status.clone()).or_insert(0usize) += 1;
    }

    problems.sort_by_key(|(rank, _)| *rank);
    problems.dedup_by(|a, b| a.1 == b.1);

    let problems_ranked: Vec<RankedProblem> = problems
        .iter()
        .map(|(severity, text)| RankedProblem { text: text.clone(), severity: *severity })
        .collect();

    StackTopology {
        subject: input.subject,
        lead,
        generated_at: input.now,
        window: input.window.clone(),
        layers: LAYERS
            .iter()
            .enumerate()
            .map(|(index, (key, label))| StackLayer { index, key: (*key).to_string(), label: (*label).to_string() })
            .collect(),
        nodes,
        edges,
        problems: problems.into_iter().map(|(_, text)| text).collect(),
        problems_ranked,
        counts,
    }
    .with_host(host_label)
}

impl StackTopology {
    /// Attach the node's address to the "You" card's neighbours as a fact on the
    /// port layer's description. Kept as a method so `build` reads top to bottom.
    fn with_host(mut self, host: Option<String>) -> Self {
        if let Some(host) = host {
            for item in &mut self.nodes {
                if item.kind == "operator" {
                    item.facts.push(fact("Dials", host.clone()));
                }
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str, parent: Option<Uuid>, role: &str, status: &str) -> SandboxFacts {
        SandboxFacts {
            id: Uuid::new_v4(),
            name: name.into(),
            pack_key: "hermes".into(),
            role: role.into(),
            summary: None,
            status: status.into(),
            health: "healthy".into(),
            parent_id: parent,
            internal_address: Some("172.30.0.5".into()),
            gateway_name: None,
            openshell: false,
        }
    }

    fn input(sandboxes: Vec<SandboxFacts>) -> StackInput {
        StackInput {
            subject: sandboxes[0].id,
            now: Utc::now(),
            window: "5m".into(),
            host: HostFacts { name: "vm200".into(), address: Some("192.168.55.50".into()), hostname: None },
            sandboxes,
            ports: vec![],
            mcp: vec![],
            supervisors: vec![],
            gateways: vec![],
            components: None,
            egress: vec![],
            dns: vec![],
        }
    }

    fn running(service: &str) -> ComponentState {
        ComponentState {
            service: service.into(),
            description: format!("{service} description"),
            container: Some(format!("nestbox-{service}-1")),
            image: None,
            state: "running".into(),
            health: None,
            started_at: None,
            restarts: 0,
            ports: serde_json::json!([]),
            cpu_pct: None,
            mem_bytes: None,
            mem_limit_bytes: None,
            pids: None,
            limits: serde_json::json!({}),
        }
    }

    fn port(sandbox_id: Uuid, kind: &str, public: i32, target: i32, listening: bool, up: bool) -> PortFacts {
        PortFacts {
            sandbox_id,
            forward_id: Uuid::new_v4(),
            purpose: if kind == "ssh" { "ssh".into() } else { "agent".into() },
            label: kind.into(),
            kind: kind.into(),
            description: None,
            target_port: target,
            public_port: public,
            dial: Some(format!("192.168.55.50:{public}")),
            url: None,
            listening: Some(listening),
            target_up: Some(up),
        }
    }

    fn kinds(topology: &StackTopology) -> Vec<&str> {
        topology.nodes.iter().map(|item| item.kind.as_str()).collect()
    }

    #[test]
    fn every_layer_is_present_even_for_a_lone_sandbox() {
        let topology = build(&input(vec![sandbox("solo", None, "agent", "running")]));
        assert_eq!(topology.layers.len(), 9);
        for kind in ["operator", "sandbox", "control_api", "runner", "egress_proxy", "internet"] {
            assert!(kinds(&topology).contains(&kind), "missing {kind}");
        }
    }

    #[test]
    fn a_stack_draws_its_lead_over_its_members() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let member = sandbox("git-01", Some(lead.id), "tool", "running");
        let (lead_id, member_id) = (lead.id, member.id);
        let topology = build(&input(vec![lead, member]));
        assert_eq!(topology.lead, lead_id);
        assert!(topology.edges.iter().any(|link| link.kind == "stack"
            && link.from == format!("sandbox:{lead_id}")
            && link.to == format!("sandbox:{member_id}")));
    }

    #[test]
    fn an_agent_without_a_supervisor_says_so_and_a_tool_does_not() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let member = sandbox("git-01", Some(lead.id), "tool", "running");
        let topology = build(&input(vec![lead, member]));
        let absent = topology.nodes.iter().filter(|item| item.kind == "absent").count();
        assert_eq!(absent, 1, "one absent supervisor, for the agent only");
    }

    #[test]
    fn a_supervisor_is_linked_to_its_sandbox_and_to_the_control_api() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let supervisor = SupervisorFacts {
            id: Uuid::new_v4(),
            name: "sup".into(),
            sandbox_id: Some(lead.id),
            status: "running".into(),
            version: Some("1".into()),
            config_dirtiness: 0,
            last_seen: Some(Utc::now()),
            checks_failing: 0,
            checks_warning: 0,
        };
        let sup_id = format!("supervisor:{}", supervisor.id);
        let mut data = input(vec![lead]);
        data.supervisors = vec![supervisor];
        let topology = build(&data);
        assert!(topology.edges.iter().any(|link| link.from == sup_id && link.kind == "supervises"));
        assert!(topology.edges.iter().any(|link| link.from == sup_id && link.to == CONTROL_API));
        let item = topology.nodes.iter().find(|item| item.id == sup_id).expect("supervisor node");
        assert_eq!(item.status, "ok");
        assert_eq!(item.logs.as_ref().and_then(|log| log.filter.as_deref()), Some("nestbox-supervisor"));
    }

    #[test]
    fn a_supervisor_that_stopped_reporting_is_stale_not_ok() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let supervisor = SupervisorFacts {
            id: Uuid::new_v4(),
            name: "sup".into(),
            sandbox_id: Some(lead.id),
            status: "running".into(),
            version: None,
            config_dirtiness: 0,
            last_seen: Some(Utc::now() - Duration::seconds(SUPERVISOR_STALE_SECONDS + 30)),
            checks_failing: 0,
            checks_warning: 0,
        };
        let mut data = input(vec![lead]);
        data.supervisors = vec![supervisor];
        let topology = build(&data);
        assert!(topology.problems.iter().any(|text| text.contains("stale")));
    }

    #[test]
    fn a_port_is_drawn_from_the_operator_to_its_sandbox_with_its_real_state() {
        let lead = sandbox("oc-01", None, "agent", "running");
        let lead_id = lead.id;
        let mut data = input(vec![lead]);
        data.ports = vec![port(lead_id, "ssh", 22201, 22, true, true), port(lead_id, "web", 22202, 3000, true, false)];
        let topology = build(&data);
        let ssh = topology.nodes.iter().find(|item| item.name.ends_with(":22201")).expect("ssh port");
        let web = topology.nodes.iter().find(|item| item.name.ends_with(":22202")).expect("web port");
        assert_eq!(ssh.status, "ok");
        assert_eq!(web.status, "warn");
        assert_eq!(web.state, "open, nothing behind it");
        assert!(topology.edges.iter().any(|link| link.from == OPERATOR && link.to == ssh.id));
        assert!(topology.edges.iter().any(|link| link.from == ssh.id && link.to == format!("sandbox:{lead_id}")));
        assert!(topology.problems.iter().any(|text| text.contains("nothing answers")));
    }

    #[test]
    fn a_closed_port_is_an_error_and_the_first_problem() {
        let lead = sandbox("oc-01", None, "agent", "running");
        let lead_id = lead.id;
        let mut data = input(vec![lead]);
        data.ports = vec![port(lead_id, "ssh", 22201, 22, false, false)];
        let topology = build(&data);
        assert!(topology.problems[0].contains("not open"));
    }

    #[test]
    fn traffic_runs_sandbox_to_proxy_to_destination_to_internet_and_blocks_are_marked() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let lead_id = lead.id;
        let mut data = input(vec![lead]);
        data.egress = vec![
            EgressFacts { sandbox_id: lead_id, host: "api.example.com".into(), port: 443, protocol: "https".into(), requests: 9, allowed: 9, denied: 0 },
            EgressFacts { sandbox_id: lead_id, host: "tracker.example".into(), port: 443, protocol: "https".into(), requests: 3, allowed: 0, denied: 3 },
        ];
        let topology = build(&data);
        let to_proxy = topology.edges.iter().find(|link| link.kind == "egress").expect("egress edge");
        assert_eq!(to_proxy.requests, 12);
        assert_eq!(to_proxy.denied, 3);
        assert_eq!(to_proxy.status, "warn");
        let blocked = topology.nodes.iter().find(|item| item.name == "tracker.example:443").expect("blocked destination");
        assert_eq!(blocked.status, "warn");
        assert!(topology.edges.iter().any(|link| link.from == blocked.id && link.to == INTERNET));
        assert!(topology.problems.iter().any(|text| text.contains("blocked")));
    }

    #[test]
    fn a_blocked_dns_name_is_an_error_node_and_the_first_problem() {
        let mut data = input(vec![sandbox("hermes-01", None, "agent", "running")]);
        data.dns = vec![DnsFacts { host: "pypi.org".into(), decision: "deny".into(), count: 3 }];
        let topology = build(&data);
        let item = topology
            .nodes
            .iter()
            .find(|item| item.kind == "dns_blocked")
            .expect("blocked DNS node");
        assert_eq!(item.status, "error");
        assert_eq!(item.layer_key, "destinations");
        assert_eq!(item.name, "pypi.org (DNS blocked)");
        assert!(item
            .facts
            .iter()
            .any(|entry| entry.label == "Decision" && entry.value == "deny"));
        assert!(item
            .facts
            .iter()
            .any(|entry| entry.label == "Blocked lookups" && entry.value == "3"));
        assert!(topology.problems[0].contains("DNS for pypi.org was blocked 3 time(s)"));
        assert_eq!(topology.problems_ranked[0].severity, 0);
        assert_eq!(topology.problems_ranked[0].text, topology.problems[0]);
    }

    #[test]
    fn a_blocked_dns_name_keeps_its_rank_through_the_sort() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let lead_id = lead.id;
        let mut data = input(vec![lead]);
        // A stopped sandbox is rank 3, so the DNS problem at rank 0 must survive
        // in front of it.
        data.dns = vec![DnsFacts { host: "pypi.org".into(), decision: "deny".into(), count: 1 }];
        data.sandboxes[0].status = "stopped".into();
        data.ports = vec![port(lead_id, "ssh", 22201, 22, true, true)];
        let topology = build(&data);
        assert!(topology.problems[0].contains("DNS for pypi.org"));
        assert_eq!(topology.problems_ranked[0].severity, 0);
        for (text, ranked) in topology.problems.iter().zip(&topology.problems_ranked) {
            assert_eq!(text, &ranked.text);
        }
    }

    #[test]
    fn an_allowed_dns_name_adds_no_node_and_no_problem() {
        let mut data = input(vec![sandbox("hermes-01", None, "agent", "running")]);
        data.dns = vec![DnsFacts { host: "api.example.com".into(), decision: "allow".into(), count: 9 }];
        let topology = build(&data);
        assert!(!kinds(&topology).contains(&"dns_blocked"));
        assert!(!topology.problems.iter().any(|text| text.contains("DNS for")));
    }

    #[test]
    fn an_empty_dns_list_changes_nothing_and_the_map_still_builds() {
        let topology = build(&input(vec![sandbox("hermes-01", None, "agent", "running")]));
        assert!(!kinds(&topology).contains(&"dns_blocked"));
        assert_eq!(topology.problems.len(), topology.problems_ranked.len());
        assert_eq!(topology.layers.len(), 9);
    }

    #[test]
    fn with_no_runner_answer_the_components_are_unknown_not_invented() {
        let topology = build(&input(vec![sandbox("solo", None, "agent", "running")]));
        let proxy = topology.nodes.iter().find(|item| item.id == PROXY).expect("proxy");
        assert_eq!(proxy.status, "unknown");
        // The API itself answered this request, so it is the one thing known.
        let api = topology.nodes.iter().find(|item| item.id == CONTROL_API).expect("api");
        assert_eq!(api.status, "ok");
    }

    #[test]
    fn a_missing_proxy_is_the_worst_problem_because_sandboxes_have_no_way_out() {
        let mut data = input(vec![sandbox("solo", None, "agent", "running")]);
        data.components = Some(vec![running("control-api"), running("sandbox-runner")]);
        let topology = build(&data);
        assert!(topology.problems.iter().any(|text| text.contains("no way out")));
    }

    #[test]
    fn components_carry_a_log_source_the_console_can_open() {
        let mut data = input(vec![sandbox("solo", None, "agent", "running")]);
        data.components = Some(vec![running("control-api"), running("sandbox-runner"), running("egress-proxy")]);
        let topology = build(&data);
        let proxy = topology.nodes.iter().find(|item| item.id == PROXY).expect("proxy");
        assert_eq!(proxy.status, "ok");
        assert_eq!(proxy.logs, Some(LogSource { kind: "component".into(), id: "egress-proxy".into(), filter: None }));
        let sandbox_node = topology.nodes.iter().find(|item| item.kind == "sandbox").expect("sandbox");
        assert_eq!(sandbox_node.logs.as_ref().map(|log| log.kind.as_str()), Some("sandbox"));
    }

    #[test]
    fn a_stopped_sandbox_is_idle_and_a_failed_one_is_an_error() {
        assert_eq!(sandbox_status("stopped", "unknown"), "idle");
        assert_eq!(sandbox_status("failed", "unhealthy"), "error");
        assert_eq!(sandbox_status("running", "unhealthy"), "error");
        assert_eq!(sandbox_status("running", "healthy"), "ok");
    }

    #[test]
    fn the_worse_of_two_statuses_wins() {
        assert_eq!(worse("ok", "warn"), "warn");
        assert_eq!(worse("error", "warn"), "error");
        assert_eq!(worse("ok", "ok"), "ok");
    }

    #[test]
    fn nodes_are_ordered_by_layer() {
        let lead = sandbox("hermes-01", None, "agent", "running");
        let lead_id = lead.id;
        let mut data = input(vec![lead]);
        data.ports = vec![port(lead_id, "ssh", 22201, 22, true, true)];
        let topology = build(&data);
        let layers: Vec<usize> = topology.nodes.iter().map(|item| item.layer).collect();
        let mut sorted = layers.clone();
        sorted.sort_unstable();
        assert_eq!(layers, sorted);
    }
}
