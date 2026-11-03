//! Egress: the decision the proxy asks for, and the evidence it leaves.
//!
//! Every outbound connection from a sandbox goes through the node's egress
//! proxy, and the proxy asks the control API what to do. The answer is one of
//! three words, and the rule that produces it is this module's `decide`. The
//! function is pure so the proxy's behaviour can be proven in a unit test
//! without a proxy.
//!
//! The order is the rule:
//!
//! 1. the rules are evaluated: the first enabled rule, by ascending priority,
//!    whose matchers (host, suffix, CIDR, port range, protocol, method, binary)
//!    all match → its action (or `audit` when the rule is audit-only);
//! 2. an open traffic window covering the request overrides that answer: a
//!    bypass window allows, an audit window allows and is recorded as a block,
//!    and `would_deny` keeps the rule verdict either way;
//! 3. a sandbox whose proxy is off (`off_allow_all`) turns a deny into an allow
//!    with the reason `allow_all`, again recording `would_deny`;
//! 4. nothing matched → deny.
//!
//! The rules go first so a window can state what would have happened without it;
//! that is what makes "allow everything for a while" evidence for the next rule
//! instead of a blind spot.
//!
//! Default deny is the property that makes the policy screen true: a sandbox
//! with no rule and no window reaches nothing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::policy::{
    Enforcement, MatchSubject, NetworkRule, RuleAction, RuleProtocol,
};

/// What the proxy asks: who wants to reach what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CheckRequest {
    pub sandbox_id: Option<Uuid>,
    pub node_id: Option<Uuid>,
    pub host: String,
    pub port: i32,
    /// `http`, `https`, `tcp`, `mcp`, `dns`.
    #[serde(default)]
    pub protocol: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub user_agent: Option<String>,
    pub mcp_tool: Option<String>,
    pub mcp_session_id: Option<String>,
}

/// The three answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    /// Let it through and log it: the rule was audit-only.
    Audit,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Audit => "audit",
        }
    }

    /// Whether the connection proceeds.
    pub fn passes(self) -> bool {
        !matches!(self, Self::Deny)
    }
}

/// Why the decision was what it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    Rule,
    Window,
    DefaultDeny,
    /// The proxy could not reach the control API and failed closed.
    BackendUnavailable,
    /// The sandbox runs with the proxy off: everything is allowed and recorded.
    AllowAll,
}

impl DecisionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Window => "window",
            Self::DefaultDeny => "default_deny",
            Self::BackendUnavailable => "backend_unavailable",
            Self::AllowAll => "allow_all",
        }
    }
}

/// The answer the proxy receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CheckResponse {
    pub decision: Decision,
    pub reason: DecisionReason,
    pub rule_id: Option<Uuid>,
    pub window_id: Option<Uuid>,
    /// A sentence the console shows verbatim.
    pub explanation: String,
    /// The hash of the rule set this decision came from, so the proxy can tell
    /// whether it is enforcing the current policy.
    pub policy_hash: String,
    /// How deeply this sandbox's egress may be observed: `forward_proxy`,
    /// `transparent` or `hybrid`. The proxy must know before it answers a
    /// CONNECT, because that answer decides tunnel versus TLS termination.
    #[serde(default = "default_egress_mode")]
    pub egress_mode: String,
    /// How long the proxy may cache this answer for the same key.
    pub cache_ttl_seconds: u32,
    /// True when the rules would have blocked this connection while a bypass
    /// or audit window (or the `off_allow_all` mode) let it through. It is what
    /// turns "allow everything for a while" into evidence for new rules.
    #[serde(default)]
    pub would_deny: bool,
}

/// The mode a response carries when the sandbox has no setting of its own.
///
/// `forward_proxy` is the safe default: the proxy sees the destination and
/// nothing inside the tunnel, which is what the operator gets unless they
/// deliberately asked for inspection.
pub fn default_egress_mode() -> String {
    "forward_proxy".to_string()
}

/// An open traffic window, as the decision function needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct OpenWindow {
    pub id: Uuid,
    pub sandbox_id: Option<Uuid>,
    /// When the window closes; `None` means it stays open until it is closed by
    /// hand (ADR 0053: an evaluation the user ends themselves).
    pub expires_at: Option<DateTime<Utc>>,
    pub reason: String,
    /// How the window treats a request it covers.
    #[serde(default)]
    pub mode: WindowMode,
    /// Which traffic the window covers.
    #[serde(default)]
    pub channel: WindowChannel,
}

/// How a window treats a request it covers.
///
/// Two modes, because they answer two different operational questions. `Bypass`
/// is "let me work for fifteen minutes". `Audit` is "show me what would be
/// blocked while nothing breaks", which is how a policy is developed against
/// real traffic instead of against guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Bypass,
    Audit,
}

impl WindowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bypass => "bypass",
            Self::Audit => "audit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bypass" => Some(Self::Bypass),
            "audit" => Some(Self::Audit),
            _ => None,
        }
    }
}

/// The firewall a window belongs to.
///
/// `All` is the broad switch; the others let an operator open one firewall
/// without opening the rest. `Openshell` exists as a word even though its switch
/// is never off: the supervisor carries the API and MCP forwarding, and a
/// sandbox with it disabled has no way to reach its tools at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowChannel {
    #[default]
    All,
    Proxy,
    Dns,
    Mcp,
    Gateway,
    Openshell,
}

impl WindowChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Proxy => "proxy",
            Self::Dns => "dns",
            Self::Mcp => "mcp",
            Self::Gateway => "gateway",
            Self::Openshell => "openshell",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "proxy" => Some(Self::Proxy),
            "dns" => Some(Self::Dns),
            "mcp" => Some(Self::Mcp),
            "gateway" => Some(Self::Gateway),
            "openshell" => Some(Self::Openshell),
            _ => None,
        }
    }

    /// Whether a window on this channel covers a request on `other`.
    ///
    /// `All` covers everything; a specific channel covers only itself. The
    /// reverse is deliberately false: a DNS window must not open the web.
    pub fn covers(self, other: Self) -> bool {
        self == Self::All || self == other
    }

    /// Whether this is a channel an operator may switch at all.
    ///
    /// Every channel is switchable now: an OpenShell window is compiled into
    /// the sandbox's own policy in audit mode (ADR 0053), so the supervisor
    /// records instead of blocking without cutting the sandbox off.
    pub fn switchable(self) -> bool {
        true
    }
}

/// The channel a request belongs to, from what the proxy reported.
///
/// A DNS request is a name lookup, JSON-RPC is MCP, and everything else is a
/// forward-proxy request. The mapping lives here so the decider, the console and
/// a test use one vocabulary.
pub fn channel_of_request(protocol: Option<&str>) -> WindowChannel {
    match protocol.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("dns") => WindowChannel::Dns,
        Some("mcp") | Some("jsonrpc") => WindowChannel::Mcp,
        _ => WindowChannel::Proxy,
    }
}

impl OpenWindow {
    /// Whether this window covers the request now.
    pub fn covers(&self, sandbox: Option<Uuid>, now: DateTime<Utc>) -> bool {
        if let Some(expires_at) = self.expires_at {
            if expires_at <= now {
                return false;
            }
        }
        match self.sandbox_id {
            None => true,
            Some(scoped) => sandbox == Some(scoped),
        }
    }

    /// Whether this window covers one request: the sandbox and the channel.
    pub fn covers_request(
        &self,
        sandbox: Option<Uuid>,
        channel: WindowChannel,
        now: DateTime<Utc>,
    ) -> bool {
        self.covers(sandbox, now) && self.channel.covers(channel)
    }
}

/// The rule set as compiled for one environment, in the order the proxy and the
/// decision function apply it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct CompiledPolicy {
    pub environment_id: Uuid,
    pub hash: String,
    pub compiled_at: DateTime<Utc>,
    pub rules: Vec<NetworkRule>,
    pub windows: Vec<OpenWindow>,
}

/// Decide what happens to one request.
///
/// The order is the rule: the rules decide first (so a window can say what the
/// answer *would* have been), then a covering window overrides that answer, and
/// a sandbox whose proxy is off allows whatever the rules would have blocked.
/// The `would_deny` flag records the rule verdict either way.
pub fn decide(
    request: &CheckRequest,
    policy: &CompiledPolicy,
    now: DateTime<Utc>,
) -> CheckResponse {
    let verdict = rule_verdict(request, policy);

    // A window is scoped to a channel: a DNS window must not open the web. The
    // mode decides whether the request passes as a bypass or passes *and* is
    // recorded as a block, which is what observation mode is for.
    let channel = channel_of_request(request.protocol.as_deref());
    if let Some(window) = policy
        .windows
        .iter()
        .find(|window| window.covers_request(request.sandbox_id, channel, now))
    {
        let decision = match window.mode {
            WindowMode::Bypass => Decision::Allow,
            WindowMode::Audit => Decision::Audit,
        };
        let open_until = match window.expires_at {
            Some(expires_at) => expires_at.format("%H:%M:%S UTC").to_string(),
            None => "until it is closed".to_string(),
        };
        let mut response = CheckResponse {
            decision,
            reason: DecisionReason::Window,
            rule_id: verdict.rule_id,
            window_id: Some(window.id),
            explanation: format!(
                "{} by a temporary {} window ({}), open {}.",
                match window.mode {
                    WindowMode::Bypass => "Allowed",
                    WindowMode::Audit => "Observation mode: allowed and recorded as a block",
                },
                window.channel.as_str(),
                window.reason,
                open_until
            ),
            policy_hash: policy.hash.clone(),
            cache_ttl_seconds: match window.expires_at {
                // A window is time-bounded; cache no longer than its remaining life.
                Some(expires_at) => ((expires_at - now).num_seconds().max(0) as u32).min(5),
                None => 5,
            },
            egress_mode: default_egress_mode(),
            would_deny: verdict.decision == Decision::Deny,
        };
        response.window_id = Some(window.id);
        return response;
    }

    verdict
}

/// Let everything through and record it: the answer for a sandbox whose proxy is
/// switched off (`off_allow_all`). A deny becomes an allow whose reason says so,
/// and `would_deny` remembers that the rules would have blocked it.
pub fn allow_everything(mut response: CheckResponse) -> CheckResponse {
    if response.decision == Decision::Deny {
        response.would_deny = true;
        response.decision = Decision::Allow;
        response.reason = DecisionReason::AllowAll;
        response.explanation = format!(
            "Proxy is off: everything is allowed and only recorded. {}",
            response.explanation
        );
    }
    response
}

/// The answer of the rules alone, without windows.
fn rule_verdict(request: &CheckRequest, policy: &CompiledPolicy) -> CheckResponse {
    let base = |decision: Decision, reason: DecisionReason, explanation: String| CheckResponse {
        decision,
        reason,
        rule_id: None,
        window_id: None,
        explanation,
        policy_hash: policy.hash.clone(),
        cache_ttl_seconds: 5,
        egress_mode: default_egress_mode(),
        would_deny: false,
    };

    let mut candidates: Vec<&NetworkRule> = policy
        .rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter(|rule| rule.sandbox_id.is_none() || rule.sandbox_id == request.sandbox_id)
        .filter(|rule| rule_matches(rule, request))
        .collect();
    candidates.sort_by_key(|rule| rule.priority);

    if let Some(rule) = candidates.first() {
        let (decision, verb) = match (rule.action, rule.enforcement) {
            (RuleAction::Allow, _) => (Decision::Allow, "Allowed"),
            (RuleAction::Deny, Enforcement::Enforced) => (Decision::Deny, "Blocked"),
            (RuleAction::Deny, Enforcement::Audit) => (Decision::Audit, "Would be blocked (audit only)"),
        };
        let mut response = base(
            decision,
            DecisionReason::Rule,
            format!(
                "{verb} by rule '{}' ({} {}{}).",
                if rule.name.is_empty() { "unnamed" } else { &rule.name },
                rule.action.as_str(),
                rule.host,
                rule.port.map(|p| format!(":{p}")).unwrap_or_default()
            ),
        );
        response.rule_id = Some(rule.id);
        return response;
    }

    base(
        Decision::Deny,
        DecisionReason::DefaultDeny,
        format!(
            "No rule allows {}:{}. Add an allow rule or open a temporary window.",
            request.host, request.port
        ),
    )
}

/// Whether one rule applies to one request. Host is a glob; port, protocol, and
/// method are exact when the rule states them and wildcards when it does not.
fn rule_matches(rule: &NetworkRule, request: &CheckRequest) -> bool {
    // The matcher form is authoritative since migration 0032. A rule that
    // arrives without matchers (a legacy caller, an old row) is read through
    // the flat fields it was written from, so both shapes mean the same thing.
    if !rule.matchers.is_empty() {
        let subject = MatchSubject {
            host: &request.host,
            port: request.port,
            protocol: request.protocol.as_deref(),
            method: request.method.as_deref(),
            // The executor is not part of the wire request yet; a binary rule
            // does not match rather than matching on a guess.
            binary: None,
        };
        return rule.matchers.iter().all(|matcher| matcher.matches(&subject));
    }

    if !host_glob_matches(&rule.host, &request.host) {
        return false;
    }
    if let Some(port) = rule.port {
        if port != request.port {
            return false;
        }
    }
    if !protocol_matches(rule.protocol, request.protocol.as_deref()) {
        return false;
    }
    if !rule.methods.is_empty() {
        let Some(method) = request.method.as_deref() else {
            return false;
        };
        if !rule
            .methods
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(method))
        {
            return false;
        }
    }
    true
}

/// A rule's protocol against the request's wire protocol.
///
/// `tcp` is the wildcard; `rest`/`http` cover both http and https; `mcp`
/// matches only what the proxy identified as MCP; `dns` is its own thing.
fn protocol_matches(rule: RuleProtocol, request: Option<&str>) -> bool {
    let request = request.unwrap_or("tcp").to_ascii_lowercase();
    match rule {
        RuleProtocol::Tcp => true,
        RuleProtocol::Rest | RuleProtocol::Http => {
            matches!(request.as_str(), "http" | "https" | "rest" | "mcp" | "jsonrpc" | "graphql")
        }
        RuleProtocol::Websocket => matches!(request.as_str(), "websocket" | "ws" | "wss"),
        RuleProtocol::Graphql => request == "graphql",
        RuleProtocol::Mcp => request == "mcp",
        RuleProtocol::Jsonrpc => matches!(request.as_str(), "jsonrpc" | "mcp"),
    }
}

/// Host globs: `*` matches any run of characters, `*.example.com` matches
/// subdomains but not the apex, and a bare `*` matches everything. Matching is
/// case-insensitive, as hostnames are.
pub fn host_glob_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == host;
    }
    glob(&pattern, &host)
}

fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark): (Option<usize>, usize) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// One line of traffic as the console shows it: the proxy's access-log entry
/// after the control API has attached the decision and the MCP facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct TrafficEvent {
    pub id: Uuid,
    pub sandbox_id: Option<Uuid>,
    /// The sandbox's name, so the environment-wide table can say which sandbox a
    /// row belongs to without a second request per row. Absent on a read that
    /// did not join `sandboxes`.
    #[serde(default)]
    pub sandbox_name: Option<String>,
    pub node_id: Option<Uuid>,
    pub ts: DateTime<Utc>,
    pub host: String,
    pub port: i32,
    pub protocol: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub status: Option<i32>,
    pub duration_ms: Option<i64>,
    pub bytes_in: i64,
    pub bytes_out: i64,
    pub decision: Decision,
    pub reason: DecisionReason,
    pub rule_id: Option<Uuid>,
    pub window_id: Option<Uuid>,
    /// Guessed from the user agent; the console says "probably".
    pub agent_hint: Option<String>,
    pub mcp_session_id: Option<String>,
    pub mcp_method: Option<String>,
    pub mcp_tool: Option<String>,
    /// Whether the proxy could read the request (it had the CA) or only saw a
    /// TLS tunnel.
    pub inspected: bool,
    pub has_body: bool,
    /// The path this request took, for the badge and the colour.
    pub channel: TrafficChannel,
    /// The listener that logged it, kept as the proxy wrote it.
    pub listener: Option<String>,
    /// The name the client asked for; the only destination a tunnel reveals.
    pub sni: Option<String>,
    /// What Envoy recognised the application protocol as (`http/1.1`, `h2`).
    pub app_protocol: Option<String>,
    /// ISO 3166-1 alpha-2 from the destination's registry record, when known.
    pub country: Option<String>,
    /// The autonomous system the destination's address belongs to, when the
    /// registry record names it.
    pub asn: Option<i64>,
    /// The program that opened the connection, when the enforcement saw it (an OpenShell
    /// supervisor knows the real executable behind every request).
    #[serde(default)]
    pub binary: Option<String>,
    /// The enforcement's own words for the decision, such as the policy that matched or the
    /// reason it refused, kept as written.
    #[serde(default)]
    pub reason_text: Option<String>,
}

/// The path one outbound request took, as the console colours it.
///
/// One value per row, because a row is one request and a request took one path.
/// The words are the proxy's (`listener`, `connect`, `sni`), translated once
/// here so the table, the filter, and the badge cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrafficChannel {
    /// A plain forward-proxy request: the method and the path are visible.
    Proxy,
    /// A CONNECT the proxy did not terminate: only the SNI is visible.
    Tunnel,
    /// A CONNECT the proxy terminated and decrypted.
    Inspected,
    /// A connection into the sandbox, not out of it.
    Inbound,
    /// The caller arrived over a tunnel interface (WireGuard, `tun0`).
    Vpn,
    /// A name lookup. It never opened a connection, so it is not one of the
    /// connection channels above; the console badges it separately because it is
    /// a different kind of event, not a different way of connecting.
    Dns,
}

impl TrafficChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proxy => "proxy",
            Self::Tunnel => "tunnel",
            Self::Inspected => "inspected",
            Self::Inbound => "inbound",
            Self::Vpn => "vpn",
            Self::Dns => "dns",
        }
    }

    /// The word the console shows on the badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Proxy => "Proxy",
            Self::Tunnel => "Tunnel",
            Self::Inspected => "Inspected",
            Self::Inbound => "Inbound",
            Self::Vpn => "VPN",
            Self::Dns => "DNS",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "proxy" => Self::Proxy,
            "tunnel" => Self::Tunnel,
            "inspected" => Self::Inspected,
            "inbound" => Self::Inbound,
            "vpn" => Self::Vpn,
            "dns" => Self::Dns,
            _ => return None,
        })
    }

    /// All channels, in the order the legend shows them.
    pub fn all() -> [Self; 6] {
        [
            Self::Proxy,
            Self::Tunnel,
            Self::Inspected,
            Self::Inbound,
            Self::Vpn,
            Self::Dns,
        ]
    }
}

/// What the proxy's access log says about the path a request took.
///
/// Kept as one struct so the derivation is a pure function of what was observed,
/// and a test can state every combination without a proxy.
#[derive(Debug, Clone, Copy, Default)]
pub struct ChannelFacts<'a> {
    /// The listener that logged the line: `proxy`, `terminated`, `inbound`, `vpn`.
    pub listener: Option<&'a str>,
    /// The line is a CONNECT, not a plain request.
    pub connect: bool,
    /// The name the client asked for. Present on a CONNECT and on a terminated
    /// request; absent from a plain HTTP/1.1 request, which carries a Host header.
    pub sni: Option<&'a str>,
    /// The caller as the proxy saw it: an address, or the interface name.
    pub client: Option<&'a str>,
}

/// Which channel a request took.
///
/// The order is the rule: a connection into the sandbox is inbound whatever else
/// it looks like; a tunnel caller is a VPN caller even when the request behind it
/// was decrypted; a terminated CONNECT is inspected; a CONNECT that was not
/// terminated is a tunnel; everything else is a plain proxy request.
pub fn channel_of(facts: &ChannelFacts<'_>) -> TrafficChannel {
    let listener = facts.listener.unwrap_or_default().trim().to_ascii_lowercase();
    if listener == "inbound" || listener == "ingress" {
        return TrafficChannel::Inbound;
    }
    if listener == "vpn" || client_is_tunnel(facts.client) {
        return TrafficChannel::Vpn;
    }
    if listener == "terminated" || listener == "inspected" {
        return TrafficChannel::Inspected;
    }
    if facts.connect {
        return TrafficChannel::Tunnel;
    }
    // A request with an SNI but no CONNECT is a TLS request the proxy accepted
    // directly; it did not have to be decrypted, because it never got a tunnel.
    TrafficChannel::Proxy
}

/// Whether the caller address names a tunnel interface.
///
/// The proxy reports an address, and the tunnel interfaces on a node are named
/// `wg0`, `tun0`, `vpn0`. Only those prefixes are accepted: guessing "not a
/// private address" would label a LAN caller a VPN caller.
fn client_is_tunnel(client: Option<&str>) -> bool {
    let Some(client) = client else {
        return false;
    };
    let name = client.trim().to_ascii_lowercase();
    ["wg", "tun", "vpn", "tailscale"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// An edge in the live map: one sandbox to one destination over a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrafficEdge {
    pub sandbox_id: Uuid,
    pub host: String,
    pub port: i32,
    pub protocol: String,
    pub requests: i64,
    pub allowed: i64,
    pub denied: i64,
    pub bytes: i64,
    pub last_seen: DateTime<Utc>,
    /// Tool names seen on this edge when it is MCP.
    pub tools: Vec<String>,
}

/// Guess the agent from a user agent string. Returns `None` rather than a
/// wrong name; the console labels any value "probably".
pub fn agent_hint(user_agent: Option<&str>) -> Option<String> {
    let ua = user_agent?.to_ascii_lowercase();
    const KNOWN: &[(&str, &str)] = &[
        ("hermes", "hermes"),
        ("claude-code", "claude-code"),
        ("claude", "claude"),
        ("opencode", "opencode"),
        ("openchamber", "openchamber"),
        ("openclaw", "openclaw"),
        ("antigravity", "antigravity"),
        ("mcp", "mcp-client"),
        ("python-requests", "python"),
        ("python-httpx", "python"),
        ("aiohttp", "python"),
        ("node", "node"),
        ("curl", "curl"),
        ("wget", "wget"),
    ];
    KNOWN
        .iter()
        .find(|(needle, _)| ua.contains(needle))
        .map(|(_, name)| (*name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn rule(host: &str, port: Option<i32>, action: RuleAction, priority: i32) -> NetworkRule {
        NetworkRule {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: format!("{} {host}", action.as_str()),
            action,
            host: host.into(),
            port,
            protocol: RuleProtocol::Tcp,
            methods: vec![],
            binaries: vec![],
            matchers: vec![],
            enforcement: Enforcement::Enforced,
            priority,
            enabled: true,
            version: 1,
        }
    }

    fn request(host: &str, port: i32) -> CheckRequest {
        CheckRequest {
            sandbox_id: Some(Uuid::nil()),
            node_id: None,
            host: host.into(),
            port,
            protocol: Some("https".into()),
            method: Some("GET".into()),
            path: Some("/".into()),
            user_agent: None,
            mcp_tool: None,
            mcp_session_id: None,
        }
    }

    fn policy(rules: Vec<NetworkRule>, windows: Vec<OpenWindow>) -> CompiledPolicy {
        CompiledPolicy {
            environment_id: Uuid::nil(),
            hash: "h".into(),
            compiled_at: Utc::now(),
            rules,
            windows,
        }
    }

    fn window(mode: WindowMode, channel: WindowChannel, expires_at: Option<DateTime<Utc>>) -> OpenWindow {
        OpenWindow {
            id: Uuid::new_v4(),
            sandbox_id: None,
            expires_at,
            reason: "testing".into(),
            mode,
            channel,
        }
    }

    #[test]
    fn an_audit_window_records_what_the_rules_would_have_done() {
        let now = Utc::now();
        let out = decide(
            &request("blocked.example", 443),
            &policy(vec![], vec![window(WindowMode::Audit, WindowChannel::All, Some(now + Duration::minutes(10)))]),
            now,
        );
        assert_eq!(out.decision, Decision::Audit);
        assert!(out.would_deny, "no rule allows the host, so it would have been denied");
    }

    #[test]
    fn a_bypass_window_over_an_allowed_host_is_not_a_would_deny() {
        let now = Utc::now();
        let r = rule("allowed.example", Some(443), RuleAction::Allow, 10);
        let rule_id = r.id;
        let out = decide(
            &request("allowed.example", 443),
            &policy(vec![r], vec![window(WindowMode::Bypass, WindowChannel::All, Some(now + Duration::minutes(5)))]),
            now,
        );
        assert_eq!(out.decision, Decision::Allow);
        assert!(!out.would_deny);
        // The window answered, but the rule that also allowed it is still named,
        // so the traffic view can say why the request was never at risk.
        assert_eq!(out.rule_id, Some(rule_id));
    }

    #[test]
    fn an_unlimited_window_stays_open_and_an_expired_one_does_not_cover() {
        let now = Utc::now();
        let unlimited = window(WindowMode::Bypass, WindowChannel::All, None);
        assert!(unlimited.covers(Some(Uuid::nil()), now + Duration::days(365)));
        let expired = window(WindowMode::Bypass, WindowChannel::All, Some(now - Duration::minutes(1)));
        assert!(!expired.covers(Some(Uuid::nil()), now));
    }

    #[test]
    fn proxy_off_turns_a_deny_into_a_recorded_allow() {
        let out = decide(&request("anything.example", 443), &policy(vec![], vec![]), Utc::now());
        let allowed = allow_everything(out);
        assert_eq!(allowed.decision, Decision::Allow);
        assert_eq!(allowed.reason, DecisionReason::AllowAll);
        assert!(allowed.would_deny);
    }

    #[test]
    fn nothing_matching_is_denied_by_default() {
        let out = decide(&request("example.com", 443), &policy(vec![], vec![]), Utc::now());
        assert_eq!(out.decision, Decision::Deny);
        assert_eq!(out.reason, DecisionReason::DefaultDeny);
        assert!(out.explanation.contains("example.com:443"));
    }

    #[test]
    fn an_allow_rule_allows_and_names_itself() {
        let r = rule("example.com", Some(443), RuleAction::Allow, 10);
        let id = r.id;
        let out = decide(&request("example.com", 443), &policy(vec![r], vec![]), Utc::now());
        assert_eq!(out.decision, Decision::Allow);
        assert_eq!(out.rule_id, Some(id));
    }

    #[test]
    fn lower_priority_number_wins() {
        let deny = rule("*.example.com", None, RuleAction::Deny, 5);
        let allow = rule("api.example.com", None, RuleAction::Allow, 50);
        let out = decide(&request("api.example.com", 443), &policy(vec![allow, deny], vec![]), Utc::now());
        assert_eq!(out.decision, Decision::Deny);
    }

    #[test]
    fn an_audit_deny_passes_but_is_marked() {
        let mut r = rule("example.com", None, RuleAction::Deny, 1);
        r.enforcement = Enforcement::Audit;
        let out = decide(&request("example.com", 443), &policy(vec![r], vec![]), Utc::now());
        assert_eq!(out.decision, Decision::Audit);
        assert!(out.decision.passes());
    }

    #[test]
    fn a_window_beats_every_rule_until_it_expires() {
        let deny = rule("*", None, RuleAction::Deny, 1);
        let deny_id = deny.id;
        let now = Utc::now();
        let window = OpenWindow {
            id: Uuid::new_v4(),
            sandbox_id: Some(Uuid::nil()),
            expires_at: Some(now + Duration::minutes(10)),
            reason: "debugging".into(),
            mode: WindowMode::Bypass,
            channel: WindowChannel::All,
        };
        let window_id = window.id;
        let out = decide(&request("anything.net", 80), &policy(vec![deny.clone()], vec![window.clone()]), now);
        assert_eq!(out.decision, Decision::Allow);
        assert_eq!(out.reason, DecisionReason::Window);
        assert_eq!(out.window_id, Some(window_id));
        // The rule said no and the window said yes: that is what `would_deny`
        // records, and the deny rule it came from is still named.
        assert!(out.would_deny);
        assert_eq!(out.rule_id, Some(deny_id));

        let later = decide(&request("anything.net", 80), &policy(vec![deny], vec![window]), now + Duration::minutes(11));
        assert_eq!(later.decision, Decision::Deny);
    }

    #[test]
    fn a_dns_window_does_not_open_the_web() {
        // The master switches are per firewall. A window that covers name
        // lookups must not also allow an HTTPS request, or "open DNS for five
        // minutes" would be "open everything".
        let now = Utc::now();
        let window = OpenWindow {
            id: Uuid::new_v4(),
            sandbox_id: Some(Uuid::nil()),
            expires_at: Some(now + Duration::minutes(5)),
            reason: "resolver check".into(),
            mode: WindowMode::Bypass,
            channel: WindowChannel::Dns,
        };
        let mut dns = request("example.com", 0);
        dns.protocol = Some("dns".into());
        let covered = decide(&dns, &policy(vec![], vec![window.clone()]), now);
        assert_eq!(covered.decision, Decision::Allow);
        assert_eq!(covered.reason, DecisionReason::Window);

        let https = decide(&request("example.com", 443), &policy(vec![], vec![window]), now);
        assert_eq!(https.decision, Decision::Deny, "a DNS window must not cover HTTPS");
    }

    #[test]
    fn an_observation_window_allows_but_is_recorded_as_a_block() {
        // Observation mode is the point of a policy review: nothing breaks and
        // the console still shows what would have been denied.
        let now = Utc::now();
        let window = OpenWindow {
            id: Uuid::new_v4(),
            sandbox_id: None,
            expires_at: Some(now + Duration::minutes(30)),
            reason: "policy review".into(),
            mode: WindowMode::Audit,
            channel: WindowChannel::All,
        };
        let out = decide(&request("example.com", 443), &policy(vec![], vec![window]), now);
        assert_eq!(out.decision, Decision::Audit);
        assert!(out.decision.passes(), "observation must not stop the workload");
        assert!(out.explanation.contains("Observation mode"));
    }

    #[test]
    fn a_channel_covers_itself_and_all_covers_everything() {
        assert!(WindowChannel::All.covers(WindowChannel::Dns));
        assert!(WindowChannel::Dns.covers(WindowChannel::Dns));
        assert!(!WindowChannel::Dns.covers(WindowChannel::Proxy));
        // Every channel is switchable now: an OpenShell window is compiled into
        // the sandbox's own policy in audit mode (ADR 0053).
        assert!(WindowChannel::Openshell.switchable());
        assert_eq!(channel_of_request(Some("dns")), WindowChannel::Dns);
        assert_eq!(channel_of_request(Some("mcp")), WindowChannel::Mcp);
        assert_eq!(channel_of_request(Some("https")), WindowChannel::Proxy);
        assert_eq!(channel_of_request(None), WindowChannel::Proxy);
    }

    #[test]
    fn a_sandbox_scoped_window_does_not_cover_another_sandbox() {
        let now = Utc::now();
        let window = OpenWindow {
            id: Uuid::new_v4(),
            sandbox_id: Some(Uuid::new_v4()),
            expires_at: Some(now + Duration::minutes(10)),
            reason: "x".into(),
            mode: WindowMode::Bypass,
            channel: WindowChannel::All,
        };
        let out = decide(&request("a.b", 1), &policy(vec![], vec![window]), now);
        assert_eq!(out.decision, Decision::Deny);
    }

    #[test]
    fn a_sandbox_scoped_rule_applies_only_to_that_sandbox() {
        let other = Uuid::new_v4();
        let mut r = rule("example.com", None, RuleAction::Allow, 1);
        r.sandbox_id = Some(other);
        let out = decide(&request("example.com", 443), &policy(vec![r], vec![]), Utc::now());
        assert_eq!(out.decision, Decision::Deny);
    }

    #[test]
    fn a_method_list_restricts_the_rule() {
        let mut r = rule("example.com", None, RuleAction::Allow, 1);
        r.methods = vec!["GET".into()];
        let mut post = request("example.com", 443);
        post.method = Some("POST".into());
        assert_eq!(decide(&post, &policy(vec![r.clone()], vec![]), Utc::now()).decision, Decision::Deny);
        assert_eq!(decide(&request("example.com", 443), &policy(vec![r], vec![]), Utc::now()).decision, Decision::Allow);
    }

    #[test]
    fn an_mcp_rule_matches_only_identified_mcp_traffic() {
        let mut r = rule("mcp.example.com", None, RuleAction::Allow, 1);
        r.protocol = RuleProtocol::Mcp;
        let https = request("mcp.example.com", 443);
        assert_eq!(decide(&https, &policy(vec![r.clone()], vec![]), Utc::now()).decision, Decision::Deny);
        let mut mcp = https.clone();
        mcp.protocol = Some("mcp".into());
        assert_eq!(decide(&mcp, &policy(vec![r], vec![]), Utc::now()).decision, Decision::Allow);
    }

    #[test]
    fn host_globs_behave_like_dns_operators_expect() {
        assert!(host_glob_matches("*", "anything"));
        assert!(host_glob_matches("example.com", "EXAMPLE.com"));
        assert!(host_glob_matches("*.example.com", "api.example.com"));
        assert!(!host_glob_matches("*.example.com", "example.com"));
        assert!(host_glob_matches("api.*.com", "api.example.com"));
        assert!(!host_glob_matches("example.com", "notexample.com"));
    }

    #[test]
    fn the_agent_hint_is_conservative() {
        assert_eq!(agent_hint(Some("hermes-agent/0.22.5")).as_deref(), Some("hermes"));
        assert_eq!(agent_hint(Some("claude-code/1.0")).as_deref(), Some("claude-code"));
        assert_eq!(agent_hint(Some("Mozilla/5.0")), None);
        assert_eq!(agent_hint(None), None);
    }
}


// ---------------------------------------------------------------------------
// "Agents only" inspection
// ---------------------------------------------------------------------------

/// Hosts an agent talks to when it signs in with an account or calls a model
/// directly, so a key cannot be put on the request by the gateway.
const AGENT_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "claude.ai",
    "platform.claude.com",
    "console.anthropic.com",
    "mcp-proxy.anthropic.com",
    "api.openai.com",
    "chatgpt.com",
    "chat.openai.com",
    "auth.openai.com",
    "generativelanguage.googleapis.com",
    "openrouter.ai",
    "api.mistral.ai",
    "api.groq.com",
    "api.x.ai",
    "api.deepseek.com",
    "api.cohere.com",
    "api.perplexity.ai",
];

/// Suffixes that are agent hosts as a family.
const AGENT_SUFFIXES: &[&str] = &[".openai.azure.com", ".anthropic.com", ".openai.com"];

/// Whether a destination is a model provider the agents-only mode reads.
///
/// The built-in list plus any hosts named in `NESTBOX_AGENT_HOSTS` (comma
/// separated; a leading dot is a suffix), so an operator can add a self-hosted
/// gateway without a release.
pub fn is_agent_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or("");
    if host.is_empty() {
        return false;
    }
    if AGENT_HOSTS.contains(&host) || AGENT_SUFFIXES.iter().any(|suffix| host.ends_with(suffix)) {
        return true;
    }
    std::env::var("NESTBOX_AGENT_HOSTS").is_ok_and(|extra| {
        extra.split(',').map(str::trim).filter(|item| !item.is_empty()).any(|item| {
            match item.strip_prefix('.') {
                Some(suffix) => host.ends_with(&format!(".{suffix}")) || host == suffix,
                None => host == item.to_ascii_lowercase(),
            }
        })
    })
}

/// The mode the proxy should act on for one destination.
///
/// `hybrid` reads TLS only towards model providers; for any other destination the
/// proxy gets `forward_proxy` and keeps a blind tunnel. The other modes are
/// unchanged.
pub fn mode_for_host(mode: &str, host: &str) -> String {
    if mode == "hybrid" && !is_agent_host(host) {
        "forward_proxy".to_string()
    } else {
        mode.to_string()
    }
}

#[cfg(test)]
mod agent_host_tests {
    use super::*;

    #[test]
    fn model_providers_are_agent_hosts_and_the_rest_is_not() {
        for host in ["api.anthropic.com", "claude.ai", "CHATGPT.com:443", "mine.openai.azure.com", "statsig.anthropic.com"] {
            assert!(is_agent_host(host), "{host}");
        }
        for host in ["github.com", "registry.npmjs.org", "evil-anthropic.com", ""] {
            assert!(!is_agent_host(host), "{host}");
        }
    }

    #[test]
    fn only_hybrid_is_narrowed() {
        assert_eq!(mode_for_host("hybrid", "github.com"), "forward_proxy");
        assert_eq!(mode_for_host("hybrid", "api.anthropic.com"), "hybrid");
        assert_eq!(mode_for_host("transparent", "github.com"), "transparent");
        assert_eq!(mode_for_host("forward_proxy", "api.anthropic.com"), "forward_proxy");
    }
}
