//! The OpenAPI document.
//!
//! Generated from one registry rather than from annotations on every handler.
//! The reason is testability: a test compares this registry against the router,
//! so a route that exists without documentation — or documentation for a route
//! that was removed — fails the build. Annotations can drift silently; a
//! comparison cannot.

use serde_json::json;

/// How an operation streams, if it streams at all.
///
/// SSE and WebSocket are both "a stream" to a browser and very different to a
/// proxy: nginx needs an `Upgrade` header for one and buffering off for the
/// other. The document says which, so the gateway config and the client agree
/// instead of guessing from the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    None,
    Sse,
    Ws,
}

impl StreamKind {
    /// Whether this operation streams at all.
    pub fn is_stream(self) -> bool {
        !matches!(self, Self::None)
    }

    /// The `x-stream` value in the document.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sse => "sse",
            Self::Ws => "ws",
        }
    }
}

/// One documented operation.
pub struct Operation {
    pub method: &'static str,
    pub path: &'static str,
    pub summary: &'static str,
    pub tag: &'static str,
    /// Minimum role, so a client can render or hide an action without guessing.
    pub role: &'static str,
    /// Whether a CSRF token is required (a session-authenticated mutation).
    pub requires_csrf: bool,
    /// Whether, and how, the operation streams.
    pub stream: StreamKind,
}

/// Every endpoint the router exposes.
///
/// Kept in one place so the document is reviewable as a list rather than
/// reconstructed from attributes spread over ten files.
pub const OPERATIONS: &[Operation] = &[
    op("GET", "/api/v1/healthz", "Liveness probe", "system", "none", false, StreamKind::None),
    op("GET", "/api/v1/readyz", "Readiness probe, including the database and the free disk", "system", "none", false, StreamKind::None),
    op("GET", "/api/v1/runtime", "Which runtime adapter is executing sandboxes", "system", "none", false, StreamKind::None),
    op("GET", "/api/v1/openapi.json", "This document", "system", "none", false, StreamKind::None),
    op("GET", "/api/schema/v1", "Alias of the OpenAPI document", "system", "none", false, StreamKind::None),
    op("GET", "/api/v1/docs", "Human-readable API entry page", "system", "none", false, StreamKind::None),
    op("POST", "/api/v1/auth/login", "Exchange credentials for a session", "auth", "none", false, StreamKind::None),
    op("POST", "/api/v1/auth/logout", "Revoke the current session", "auth", "viewer", true, StreamKind::None),
    op("POST", "/api/v1/auth/csrf", "A fresh CSRF token for the current session (the old one stops working)", "auth", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/auth/me", "The authenticated identity", "auth", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/users", "Users of this environment, with role and owned sandboxes (admin)", "users", "admin", false, StreamKind::None),
    op("POST", "/api/v1/users", "Create a user (admin)", "users", "admin", true, StreamKind::None),
    op("PUT", "/api/v1/users/{id}", "Change a user's name, role or status (admin)", "users", "admin", true, StreamKind::None),
    op("DELETE", "/api/v1/users/{id}", "Take a user out of this environment (admin)", "users", "admin", true, StreamKind::None),
    op("POST", "/api/v1/users/{id}/password", "Set a user's password and end their sessions (admin)", "users", "admin", true, StreamKind::None),
    op("GET", "/api/v1/environments", "Environments this user may access", "environments", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/environments/current", "Summary of the active environment", "environments", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes", "List sandboxes with cursor paging and facets", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes", "Create a sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}", "Read a sandbox and its runtime facts", "sandboxes", "viewer", false, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}", "Delete a sandbox and its container", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/actions", "Start, stop, or restart a sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/logs", "Recent log lines of a sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/exec", "Run one command inside a sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/inference", "Model requests made by a sandbox", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/connect", "How to reach a sandbox: node address, hostname, ports, SSH", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/stack-topology", "The stack a sandbox belongs to, from the operator to the internet", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/system/components", "State of the appliance's own containers", "system", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/system/components/{service}/logs", "Recent log lines of an appliance component", "system", "operator", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/ssh-access", "Operator keys and temporary logins of a sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/ssh-keys", "Add an operator's public SSH key to a sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/ssh-keys/{key}", "Remove an operator key", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/ssh-credentials", "Issue a temporary SSH user name and password (shown once)", "sandboxes", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/ssh-credentials/{credential}", "Revoke a temporary login", "sandboxes", "operator", true, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/resources", "Change a sandbox's memory, CPU and process limits, live when it runs", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/host/load", "CPU, memory, disk and promised resources of the host", "system", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/agent-config", "Write the agent's configuration (model endpoint, proxy) into a running sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/recreate", "Replace the container with one that has the current mounts (files outside persistent roots are lost)", "sandboxes", "operator", true, StreamKind::None),
    op("PUT", "/api/v1/nodes/{id}/endpoint", "Set the address and hostname clients dial on a node", "nodes", "admin", true, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/stack", "Put a sandbox into a stack or take it out", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/rules", "Network rules attached to a sandbox", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/allow", "Let one destination through for this sandbox, in one step", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/policies", "List policies and the global override", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/policies", "Create a policy", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/policies/global", "The active global override", "policies", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/policies/{id}", "Read a policy", "policies", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/policies/{id}", "Replace a policy document", "policies", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/policies/{id}", "Delete a policy", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/policies/{id}/diff", "Diff against an earlier revision", "policies", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/policies/{id}/versions", "The revision history of a policy, newest first", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/policies/{id}/clone", "Copy a policy under a new name", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/policies/{id}/assign", "Assign a policy to several sandboxes", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/network-rules/copy", "Copy rules to other sandboxes or to all of them", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/policy-templates", "List the preset library", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/policy-templates", "Save a preset", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/policy-templates/{id}/instantiate", "Turn a preset into a policy", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/policy-schema", "The document schema the editor renders from", "policies", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/policy", "The policy a sandbox carries", "policies", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/policy", "Assign a policy to a sandbox", "policies", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/policy", "Remove a sandbox's policy", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/network-rules", "List network rules", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/network-rules", "Create a network rule", "policies", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/network-rules/{id}", "Delete a network rule", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/dns-rules", "List DNS rules", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/dns-rules", "Create a DNS rule", "policies", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/dns-rules/{id}", "Delete a DNS rule", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/egress-mode", "One sandbox's egress mode", "policies", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/egress-mode", "Set one sandbox's egress mode", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/control-overview", "Every sandbox's controls at a glance", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/control", "Everything the workbox shows for one sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/agents/auth", "The sign-in mode of every switchable agent", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/agents/activity", "What the agents are doing now", "sandboxes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/agents/{agent}/auth", "Switch an agent between subscription and gateway", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/agents/{agent}/restart", "Restart one agent", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/presence", "Who is connected to the sandbox now", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/models", "The models this sandbox may use", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/move-preflight", "Whether a move can work, before anything stops", "sandboxes", "viewer", false, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/terminal-tabs/{tab}", "Close one terminal tab for everyone", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/rule-drafts/bulk", "Draft rules from what was denied or only observed", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/models/{key}/warm", "Load a model and answer one tiny request", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/model-grants", "Model grants", "models", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/model-grants", "Grant a model to the environment, a user or a sandbox", "models", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/model-grants/{id}", "Revoke a model grant", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/internal/accounting/models/{sandbox}", "The models a sandbox may use, for the accounting proxy", "internal", "admin", false, StreamKind::None),
    op("POST", "/api/v1/nodes/connect", "Join an existing machine by address and SSH", "nodes", "admin", true, StreamKind::None),
    op("GET", "/api/v1/nodes/tunnels", "The WireGuard tunnel of every node", "nodes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/nodes/{id}/egress-lock", "Choose how a node reaches the internet", "nodes", "admin", true, StreamKind::None),
    op("GET", "/api/v1/node/egress-lock", "The lock a node should apply", "internal", "admin", false, StreamKind::None),
    op("GET", "/api/v1/rule-drafts", "Pending rule drafts", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/rule-drafts", "Draft a rule from an event", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/rule-drafts/{id}/approve", "Approve a draft and enforce it", "policies", "operator", true, StreamKind::None),
    op("POST", "/api/v1/rule-drafts/{id}/reject", "Reject a draft", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/traffic-windows", "The traffic windows in force, with mode and channel", "policies", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/traffic-windows", "Open a temporary traffic window", "policies", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/traffic-windows/{id}", "Close a traffic window early", "policies", "operator", true, StreamKind::None),
    op("GET", "/api/v1/telemetry/events", "Recent telemetry events", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/telemetry/facets", "Event counts per kind", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/telemetry/stream", "Live telemetry stream", "telemetry", "viewer", false, StreamKind::Sse),
    op("POST", "/api/v1/telemetry/ingest", "Record an event", "telemetry", "operator", true, StreamKind::None),
    op("POST", "/api/v1/telemetry/seed-demo", "Generate sample events for a fresh console", "telemetry", "operator", true, StreamKind::None),
    op("GET", "/api/v1/events/stream", "Live state-change stream", "telemetry", "viewer", false, StreamKind::Sse),
    op("GET", "/api/v1/gateways", "List gateways", "cluster", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/gateways/{id}", "Read a gateway and its drift", "cluster", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/gateways/{id}/settings", "Replace gateway configuration", "cluster", "admin", true, StreamKind::None),
    op("POST", "/api/v1/gateways/{id}/sync", "Push configuration and clear drift", "cluster", "admin", true, StreamKind::None),
    op("GET", "/api/v1/supervisors", "List supervisors and their config dirtiness", "cluster", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/supervisors", "Register a supervisor for a sandbox", "cluster", "operator", true, StreamKind::None),
    op("PATCH", "/api/v1/supervisors/{id}", "Bind a supervisor to a sandbox", "cluster", "operator", true, StreamKind::None),
    op("POST", "/api/v1/internal/supervisors/{id}/report", "A supervisor reports what it measured (supervisor token)", "cluster", "node", false, StreamKind::None),
    op("GET", "/api/v1/topology", "The cluster graph", "cluster", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/providers", "List provider endpoint profiles", "providers", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/providers", "Register a provider profile", "providers", "operator", true, StreamKind::None),
    op("GET", "/api/v1/providers/{id}", "Read a provider profile", "providers", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/providers/{id}", "Change a provider profile", "providers", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/providers/{id}", "Remove a provider profile", "providers", "operator", true, StreamKind::None),
    op("POST", "/api/v1/providers/{id}/probe", "Probe a provider and its credential", "providers", "operator", true, StreamKind::None),
    op("GET", "/api/v1/secrets", "Stored keys, without their values", "providers", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/secrets", "Store a key (write-only) or rotate its value", "providers", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/secrets/{id}", "Delete a stored key that no provider uses", "providers", "operator", true, StreamKind::None),
    op("GET", "/api/v1/models", "Model catalog with per-machine fit", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/installed", "Installed models", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/hardware", "Detected hardware", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/facets", "Catalog counts per family, publisher, runtime, and verdict", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/requirements", "One model's requirements and verdict", "models", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/models/{key}/install", "Install or update a model on a node", "models", "operator", true, StreamKind::None),
    op("POST", "/api/v1/models/{key}/uninstall", "Remove a model from a node", "models", "operator", true, StreamKind::None),
    op("PATCH", "/api/v1/models/{key}/install", "Edit an installed model: reserved memory, on or off", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/models/{key}/logs", "The served model's recent log lines", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/{key}/metrics", "Sample the served model and return its stored series", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/models/installs", "Model installs per node with reservations", "models", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/models/sync", "Refresh the catalog from Hugging Face, Ollama, and NVIDIA", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/cuda-x/components", "CUDA-X component catalog", "models", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/cuda-x/installs", "CUDA-X installations per node with warnings", "models", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/cuda-x/components/{key}/assign", "Assign a CUDA-X component to a node", "models", "operator", true, StreamKind::None),
    op("POST", "/api/v1/cuda-x/components/{key}/install", "Install a CUDA-X component on a node, warnings included", "models", "operator", true, StreamKind::None),
    op("POST", "/api/v1/cuda-x/components/{key}/remove", "Remove a CUDA-X assignment from a node", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/nodes", "Nodes with capacity, reservations, and sandbox counts", "nodes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/nodes/{id}", "One node with its budget", "nodes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/nodes/{id}/sandboxes", "Sandboxes on a node", "nodes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/nodes/{id}/display", "Rename or describe a node", "nodes", "admin", true, StreamKind::None),
    op("POST", "/api/v1/nodes/{id}/probe", "Probe a node's hardware through its runner", "nodes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/baselines", "Base operating systems a recipe can start from", "builder", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/agent-packs", "Agent packs with baseline compatibility", "builder", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/tool-catalog", "Tools with their apt and dnf names", "builder", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/recipes", "List recipes", "builder", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/recipes", "Create a draft recipe", "builder", "operator", true, StreamKind::None),
    op("GET", "/api/v1/recipes/{id}", "Read a recipe", "builder", "viewer", false, StreamKind::None),
    op("PATCH", "/api/v1/recipes/{id}", "Edit a recipe; bumps the revision", "builder", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/recipes/{id}", "Delete a recipe", "builder", "operator", true, StreamKind::None),
    op("POST", "/api/v1/recipes/{id}/build", "Build the recipe's image as a job", "builder", "operator", true, StreamKind::None),
    op("POST", "/api/v1/recipes/{id}/instantiate", "Create a sandbox from a built recipe", "builder", "operator", true, StreamKind::None),
    op("POST", "/api/v1/recipes/{id}/apply-template", "Apply an image template's selection to a recipe", "builder", "operator", true, StreamKind::None),
    op("GET", "/api/v1/packages", "Search the live package catalogue of a baseline's distribution", "builder", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/image-templates", "List image templates this actor may use", "builder", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/image-templates", "Save a builder selection as a template", "builder", "operator", true, StreamKind::None),
    op("GET", "/api/v1/image-templates/default", "The environment's default image template", "builder", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/image-templates/{id}", "Read an image template", "builder", "viewer", false, StreamKind::None),
    op("PATCH", "/api/v1/image-templates/{id}", "Replace an image template", "builder", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/image-templates/{id}", "Delete an image template", "builder", "operator", true, StreamKind::None),
    op("GET", "/api/v1/persistent-roots", "Persistent roots and where they are mounted", "persistence", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/persistent-roots", "Create a persistent root on a node", "persistence", "operator", true, StreamKind::None),
    op("GET", "/api/v1/persistent-roots/{id}", "One root with its mounts", "persistence", "viewer", false, StreamKind::None),
    op("DELETE", "/api/v1/persistent-roots/{id}", "Archive or delete a root", "persistence", "operator", true, StreamKind::None),
    op("POST", "/api/v1/persistent-roots/{id}/mount", "Mount a root into a sandbox", "persistence", "operator", true, StreamKind::None),
    op("POST", "/api/v1/persistent-roots/{id}/dismount", "Dismount a root from a sandbox", "persistence", "operator", true, StreamKind::None),
    op("GET", "/api/v1/persistent-roots/{id}/usage", "Measured disk usage of a root", "persistence", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/persistent-roots/{id}/backups", "The backups of one persistent root", "persistence", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/persistent-roots/{id}/backups", "Back up a persistent root now", "persistence", "operator", true, StreamKind::None),
    op("GET", "/api/v1/backups", "Every root backup, newest first", "persistence", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/backups/{id}/restore", "Restore a backup into a root", "persistence", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/backups/{id}", "Delete a backup and its archive", "persistence", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/agents", "Agents inside a sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/agents", "Add an agent to a sandbox", "sandboxes", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/agents/{instance}", "Remove an agent, keeping or dropping its data", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/jobs", "List jobs", "jobs", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/jobs/{id}", "Read a job and its progress", "jobs", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/jobs/{id}/logs", "A job's log", "jobs", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/schedules", "List schedules", "jobs", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/schedules/{id}/run", "Run a schedule now", "jobs", "operator", true, StreamKind::None),
    // Control Center (Wave 0 stubs)
    op("GET", "/api/v1/inventory/tree", "The inventory tree: organisation, data centers, sites, nodes, sandboxes", "inventory", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/search", "Search sandboxes, nodes, models, policies and more", "inventory", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/bulk", "Apply one action to many sandboxes as a job", "sandboxes", "member", true, StreamKind::None),
    op("POST", "/api/v1/models/{key}/install-bulk", "Install one model on many nodes as a job", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/model-clusters", "List model clusters", "models", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/model-clusters", "Create a model cluster and its provider profile", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/model-clusters/{id}", "Read a model cluster with its members", "models", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/model-clusters/{id}", "Rename a cluster or change its strategy", "models", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/model-clusters/{id}", "Delete a cluster and its provider profile", "models", "operator", true, StreamKind::None),
    op("POST", "/api/v1/model-clusters/{id}/members", "Add an install to a cluster", "models", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/model-clusters/{id}/members/{install_id}", "Remove an install from a cluster", "models", "operator", true, StreamKind::None),
    op("GET", "/api/v1/internal/accounting/clusters/{id}", "A cluster's members for the accounting proxy (bridge token)", "models", "node", false, StreamKind::None),
    op("GET", "/api/v1/nodes/{id}/summary", "One node with load, sandboxes, installs, images and tunnel", "nodes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/nodes/{id}/drain", "Move or stop every sandbox of a node as a job", "nodes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/nodes/quick", "Provision or join a node, optionally with a model, as a job", "nodes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/alerts", "Refused destinations nobody answered yet", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/mounts", "The mounts a sandbox has and could have", "sandboxes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/mounts", "Make a sandbox's mount set equal the body", "sandboxes", "member", true, StreamKind::None),
    op("POST", "/api/v1/policies/preview", "Compile a policy spec that is not saved yet", "policies", "viewer", true, StreamKind::None),
    op("GET", "/api/v1/policies/{id}/compiled", "The compiled OpenShell document of a stored policy", "policies", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/quick", "One-click control of a sandbox: allow all, egress mode, recording", "sandboxes", "member", true, StreamKind::None),
    op("GET", "/api/v1/permissions", "Who has which role in a scope", "users", "admin", false, StreamKind::None),
    op("PUT", "/api/v1/permissions", "Give a user a role in a scope", "users", "admin", true, StreamKind::None),
    op("DELETE", "/api/v1/permissions", "Take a user's role in a scope away", "users", "admin", true, StreamKind::None),
    op("GET", "/api/v1/users/me/tours", "List the first-run tours the user has seen", "users", "viewer", true, StreamKind::None),
    op("PUT", "/api/v1/users/me/tours/{tour}", "Mark a first-run tour as seen", "users", "viewer", true, StreamKind::None),
    op("GET", "/api/v1/audit-events", "Recent audit events", "audit", "viewer", false, StreamKind::None),
    // Admin Center: resource history, processes, inventory
    op("GET", "/api/v1/sandboxes/{id}/metrics", "CPU, memory, network and disk over a window", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/supervisor", "The sandbox's OpenShell supervisor: boundary state, policy, recent log", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/login-sessions", "How each running sandbox's agent is signed in, and who is connected", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/processes", "The sandbox's process table, read live", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/live/{kind}", "A live view of the sandbox: connections", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/desktop", "The sandbox desktop (VNC over WebSocket)", "sandboxes", "operator", false, StreamKind::Ws),
    op("GET", "/api/v1/sandboxes/{id}/display", "The monitor layout of a desktop sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/display", "Apply a monitor layout to a desktop sandbox", "sandboxes", "member", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/inventory", "The last inventory snapshot and its age", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/inventory/refresh", "Take a new inventory snapshot", "sandboxes", "operator", true, StreamKind::None),
    op("POST", "/api/v1/internal/metrics/ingest", "Record a batch of resource samples (node token)", "sandboxes", "node", false, StreamKind::None),
    // Admin Center: terminal, SSH, ports, sessions
    op("GET", "/api/v1/sandboxes/{id}/terminal", "Attach an interactive terminal over WebSocket", "sandboxes", "operator", false, StreamKind::Ws),
    op("GET", "/api/v1/sandboxes/{id}/terminal-tabs", "The terminal tabs this user keeps in the sandbox, with their shell state", "sandboxes", "operator", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/terminal-sessions", "Who is attached to which terminal tab", "sandboxes", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/terminal-tabs", "Replace the terminal tab set", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/ssh-key", "The SSH key record, never the private key", "sandboxes", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/ssh-key/download", "Download the private key, once", "sandboxes", "operator", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/ssh-key/rotate", "Replace the SSH key pair", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/ports", "Ports published on the node for this sandbox", "sandboxes", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/ports", "Publish another sandbox port from the node pool", "sandboxes", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/ports/{forward_id}", "Withdraw a published port", "sandboxes", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/sessions", "Terminal and SSH sessions, without content", "sandboxes", "viewer", false, StreamKind::None),
    // Admin Center: traffic, map, effective policies, MCP calls
    op("GET", "/api/v1/traffic", "Traffic across every sandbox of the environment, with counts", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/traffic/decisions", "Denied destinations, grouped, for the decisions view", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/traffic", "Egress traffic rows for a window", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/traffic/{event_id}/body", "The redacted request and response body", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/map", "Aggregated egress edges for the live map", "telemetry", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/mcp/calls", "MCP calls the proxy saw", "mcp", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/policies/effective", "Rules that apply to this sandbox, with deployment state", "policies", "viewer", false, StreamKind::None),
    // MCP registry and sandbox attachments
    op("GET", "/api/v1/mcp/servers", "MCP servers in the registry", "mcp", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/mcp/servers", "Register an MCP server", "mcp", "operator", true, StreamKind::None),
    op("GET", "/api/v1/mcp/servers/{key}", "Read an MCP server definition", "mcp", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/mcp/servers/{key}", "Replace an MCP server definition", "mcp", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/mcp/servers/{key}", "Remove an MCP server definition", "mcp", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/mcp/servers", "MCP servers attached to a sandbox", "mcp", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/mcp/servers", "Attach a registry server to a sandbox", "mcp", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/sandboxes/{id}/mcp/servers/{key}", "Detach a server from a sandbox", "mcp", "operator", true, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/mcp/servers/{key}/discover", "Ask a server for its tool list, through the policy check", "mcp", "operator", true, StreamKind::None),
    // Internal: node-token endpoints for the proxy and the runner
    op("POST", "/api/v1/internal/policy/check", "Decide one egress request (node token)", "policies", "node", false, StreamKind::None),
    op("POST", "/api/v1/internal/telemetry/batch", "Ingest a proxy access-log batch (node token)", "telemetry", "node", false, StreamKind::None),
    op("POST", "/api/v1/internal/media/render", "Render an image, video or music asset through the selected model (media token)", "media", "node", false, StreamKind::None),
    op("GET", "/api/v1/internal/media/info", "Whether the media bridge is available (media token)", "media", "node", false, StreamKind::None),
    op("GET", "/api/v1/internal/media/renders/{id}/asset", "The bytes of one render, for the media bridge (media token)", "media", "node", false, StreamKind::None),
    // Media settings and renders
    op("GET", "/api/v1/accounting/overview", "Organisation or sandbox totals for a timeframe", "accounting", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/accounting/requests", "Recent proxied requests with tokens and cost", "accounting", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/accounting/usage", "Model usage grouped by provider, model, sandbox, owner, site or day", "accounting", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/accounting/prices", "The price list and the used models without a price", "accounting", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/accounting/prices", "Set a model price; earlier unpriced calls are priced", "accounting", "admin", true, StreamKind::None),
    op("GET", "/api/v1/accounting/settings", "Accounting retention settings", "accounting", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/accounting/settings", "Set the accounting retention", "accounting", "admin", true, StreamKind::None),
    op("POST", "/api/v1/accounting/purge", "Delete raw accounting rows older than a number of days", "accounting", "admin", true, StreamKind::None),
    op("POST", "/api/v1/internal/accounting/record", "Record one proxied request (accounting token)", "accounting", "node", false, StreamKind::None),
    op("POST", "/api/v1/internal/privacy/inspect", "Privacy decision for one proxied body (accounting token)", "accounting", "node", false, StreamKind::None),
    op("GET", "/api/v1/privacy/settings", "Content privacy settings for the environment or one sandbox", "privacy", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/privacy/settings", "Replace the content privacy policy", "privacy", "operator", true, StreamKind::None),
    op("DELETE", "/api/v1/privacy/settings/{sandbox_id}", "Drop a sandbox override so it inherits the environment default", "privacy", "operator", true, StreamKind::None),
    op("POST", "/api/v1/privacy/inspect", "Run the policy over one piece of text and report what would be sent", "privacy", "operator", true, StreamKind::None),
    op("GET", "/api/v1/privacy/incidents", "Recent content privacy incidents for one sandbox", "privacy", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/privacy/overview", "DLP incidents grouped by detector, action, sandbox or day", "privacy", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/privacy/log", "The newest DLP incidents with masked excerpts", "privacy", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/privacy/changes", "Who changed the DLP policy, when, and from what to what", "privacy", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/chats/turns", "Model calls of this environment as chat turns", "chat", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/chats/groups", "The same turns grouped by session, model, provider, day, sandbox or owner", "chat", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/chats/turns/{id}", "One turn with the findings of its request", "chat", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/chats/stream", "Turns starting, answers growing, turns stored", "chat", "viewer", false, StreamKind::Sse),
    op("POST", "/api/v1/live/watch", "Record that a live view was opened", "chat", "viewer", true, StreamKind::None),
    op("GET", "/api/v1/privacy/exceptions", "Findings allowed on purpose", "privacy", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/privacy/exceptions", "Allow one finding on purpose", "privacy", "viewer", true, StreamKind::None),
    op("DELETE", "/api/v1/privacy/exceptions/{id}", "Remove an exception", "privacy", "viewer", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/quick-allow", "Allow rules with an end", "policy", "viewer", false, StreamKind::None),
    op("POST", "/api/v1/sandboxes/{id}/quick-allow", "Allow one destination for a while", "policy", "viewer", true, StreamKind::None),
    op("POST", "/api/v1/internal/privacy/refusal", "A refusal without a finding, reported by the proxy", "privacy", "system", true, StreamKind::None),
    op("POST", "/api/v1/internal/chats/live", "A live report of a model request", "chat", "system", true, StreamKind::None),
    op("POST", "/api/v1/privacy/purge", "Delete incidents older than a number of days", "privacy", "admin", true, StreamKind::None),
    op("GET", "/api/v1/media/settings", "Environment default and the media providers this person may see", "media", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/media/users", "Every person with their effective media release", "media", "admin", false, StreamKind::None),
    op("PUT", "/api/v1/media/users/{id}", "Release media providers to one person, or take them back", "media", "admin", true, StreamKind::None),
    op("PUT", "/api/v1/media/settings", "Set the environment media default and allow-list", "media", "admin", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/media", "Effective and overridden media models for a sandbox", "media", "viewer", false, StreamKind::None),
    op("PUT", "/api/v1/sandboxes/{id}/media", "Choose a sandbox's own media models from the allowed set", "media", "operator", true, StreamKind::None),
    op("GET", "/api/v1/sandboxes/{id}/media/renders", "Recent media renders for a sandbox", "media", "viewer", false, StreamKind::None),
    op("GET", "/api/v1/media/renders/{id}/asset/{name}", "One rendered image, video or track", "media", "viewer", false, StreamKind::None),
    op("DELETE", "/api/v1/media/renders/{id}", "Delete one render and its asset", "media", "admin", true, StreamKind::None),
    op("POST", "/api/v1/media/purge", "Delete renders older than the given number of days", "media", "admin", true, StreamKind::None),
];

const fn op(
    method: &'static str,
    path: &'static str,
    summary: &'static str,
    tag: &'static str,
    role: &'static str,
    requires_csrf: bool,
    stream: StreamKind,
) -> Operation {
    Operation {
        method,
        path,
        summary,
        tag,
        role,
        requires_csrf,
        stream,
    }
}

/// Build the OpenAPI document.
pub fn document() -> serde_json::Value {
    let mut paths = serde_json::Map::new();

    for operation in OPERATIONS {
        let entry = paths
            .entry(operation.path.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));

        // Built as a map up front rather than patched through a fallible
        // accessor, so there is no unwrap on a value this function constructs.
        let mut fields = serde_json::Map::new();
        fields.insert("summary".into(), json!(operation.summary));
        fields.insert("tags".into(), json!([operation.tag]));
        fields.insert("x-roles".into(), json!([operation.role]));
        fields.insert("x-requires-csrf".into(), json!(operation.requires_csrf));

        if operation.stream.is_stream() {
            fields.insert("x-stream".into(), json!(operation.stream.as_str()));
            fields.insert(
                "responses".into(),
                json!({ "200": { "description": "A stream of server-sent events" } }),
            );
        } else {
            fields.insert(
                "responses".into(),
                json!({ "200": { "description": "Success" } }),
            );
        }

        let body = serde_json::Value::Object(fields);

        if let Some(object) = entry.as_object_mut() {
            object.insert(operation.method.to_lowercase(), body);
        }
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Nestbox Control API",
            "version": "2.0.0",
            "description": "API-first control plane for governed AI-agent sandboxes. \
                            The /api/v1/ prefix is frozen; a breaking change gets a new prefix."
        },
        "servers": [{ "url": "/" }],
        "components": {
            "securitySchemes": {
                "session": { "type": "apiKey", "in": "cookie", "name": "nestbox_session" },
                "token": { "type": "http", "scheme": "bearer" }
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": {
                            "type": "object",
                            "properties": {
                                "code": { "type": "string" },
                                "message": { "type": "string" },
                                "details": { "type": "object" },
                                "request_id": { "type": "string" }
                            },
                            "required": ["code", "message"]
                        }
                    }
                }
            }
        },
        "security": [{ "session": [] }, { "token": [] }],
        "tags": [
            { "name": "system" }, { "name": "auth" }, { "name": "environments" },
            { "name": "sandboxes" }, { "name": "policies" }, { "name": "telemetry" },
            { "name": "cluster" }, { "name": "providers" }, { "name": "models" }, { "name": "jobs" },
            { "name": "nodes" }, { "name": "builder" }, { "name": "persistence" }, { "name": "mcp" },
            { "name": "media" },
            { "name": "audit" }
        ],
        "paths": paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_has_a_unique_method_and_path() {
        let mut seen = std::collections::BTreeSet::new();
        for operation in OPERATIONS {
            let key = format!("{} {}", operation.method, operation.path);
            assert!(seen.insert(key.clone()), "duplicate operation: {key}");
        }
    }

    #[test]
    fn paths_are_versioned_except_the_schema_alias() {
        for operation in OPERATIONS {
            assert!(
                operation.path.starts_with("/api/v1/")
                    || operation.path.starts_with("/api/schema/"),
                "{} is outside the versioned prefix",
                operation.path
            );
        }
    }

    #[test]
    fn every_mutation_declares_csrf_requirements() {
        // A mutation that claims no CSRF need is either token-authenticated or a
        // mistake, and the list is small enough to enumerate deliberately.
        //
        // The internal routes are authenticated by a node token, not a session
        // cookie, so there is no browser to protect and no CSRF token to send.
        // Listing them here means a fourth one cannot appear without a decision.
        const NODE_TOKEN_MUTATIONS: &[&str] = &[
            "/api/v1/internal/metrics/ingest",
            "/api/v1/internal/policy/check",
            "/api/v1/internal/supervisors/{id}/report",
            "/api/v1/internal/telemetry/batch",
            "/api/v1/internal/media/render",
            "/api/v1/internal/accounting/record",
            "/api/v1/internal/privacy/inspect",
        ];
        for operation in OPERATIONS {
            if matches!(operation.method, "POST" | "PUT" | "PATCH" | "DELETE") {
                // Login has no session yet. Issuing a CSRF token is how a browser that lost
                // its token gets one: it needs the session cookie (SameSite=Lax, so a
                // cross-site POST does not carry it) and answers only in the body.
                if operation.path == "/api/v1/auth/login" || operation.path == "/api/v1/auth/csrf" {
                    continue;
                }
                if NODE_TOKEN_MUTATIONS.contains(&operation.path) {
                    continue;
                }
                assert!(
                    operation.requires_csrf,
                    "{} {} must declare CSRF requirements",
                    operation.method, operation.path
                );
            }
        }
    }

    #[test]
    fn the_node_token_routes_are_the_ones_that_skip_csrf() {
        const NODE_TOKEN_MUTATIONS: &[&str] = &[
            "/api/v1/internal/metrics/ingest",
            "/api/v1/internal/policy/check",
            "/api/v1/internal/supervisors/{id}/report",
            "/api/v1/internal/telemetry/batch",
            "/api/v1/internal/media/render",
            "/api/v1/internal/accounting/record",
            "/api/v1/internal/privacy/inspect",
        ];
        for path in NODE_TOKEN_MUTATIONS {
            let operation = OPERATIONS
                .iter()
                .find(|operation| operation.method == "POST" && operation.path == *path)
                .unwrap_or_else(|| panic!("{path} is not a documented POST"));
            assert!(
                !operation.requires_csrf,
                "{path} is node-token authenticated and needs no CSRF token"
            );
            assert_eq!(operation.role, "node", "{path} runs as a node, not a user");
        }
    }

    #[test]
    fn the_document_contains_every_registered_path() {
        let document = document();
        let paths = document["paths"].as_object().expect("paths");
        for operation in OPERATIONS {
            let entry = paths
                .get(operation.path)
                .unwrap_or_else(|| panic!("missing path {} in document", operation.path));
            assert!(
                entry.get(operation.method.to_lowercase()).is_some(),
                "missing {} {} in document",
                operation.method,
                operation.path
            );
        }
    }

    #[test]
    fn streams_are_marked() {
        let document = document();
        let stream = &document["paths"]["/api/v1/telemetry/stream"]["get"];
        assert_eq!(stream["x-stream"], "sse");

        // The terminal is a WebSocket, not SSE: a gateway needs different
        // settings for it, and the document has to say which.
        let terminal = &document["paths"]["/api/v1/sandboxes/{id}/terminal"]["get"];
        assert_eq!(terminal["x-stream"], "ws");

        // A plain request must not claim to stream at all.
        let plain = &document["paths"]["/api/v1/sandboxes/{id}/metrics"]["get"];
        assert!(plain.get("x-stream").is_none());
    }
}
