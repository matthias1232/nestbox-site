//! Policy documents, scopes, sections, and network rules.
//!
//! Two facts drive this model (ADR 0007, ADR 0008):
//!
//! 1. The runtime has exactly two scopes: a sandbox policy, and one global
//!    override that replaces every sandbox policy. There is no third scope, so
//!    adding one to this type would create a state the runtime cannot enforce.
//! 2. Only the network section is hot-reloadable. The other three are fixed when
//!    the workload starts, and the API says so rather than letting a save look
//!    like a live change.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::egress::host_glob_matches;

/// Which sandboxes a policy applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyScope {
    /// Applies where assigned via `sandbox_policies`.
    Sandbox,
    /// Replaces every sandbox policy and blocks sandbox-level edits until
    /// deleted.
    Global,
}

impl PolicyScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Global => "global",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sandbox" => Some(Self::Sandbox),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

/// The four sections of a policy document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicySection {
    FilesystemPolicy,
    Landlock,
    Process,
    NetworkPolicies,
}

impl PolicySection {
    /// The key in the document.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemPolicy => "filesystem_policy",
            Self::Landlock => "landlock",
            Self::Process => "process",
            Self::NetworkPolicies => "network_policies",
        }
    }

    /// Whether a change to this section takes effect on a running sandbox.
    ///
    /// This is the single source of truth for the warning the console shows and
    /// for the outcome the API reports. Encoding it here means the answer cannot
    /// differ between the editor and the save.
    pub fn is_hot_reloadable(self) -> bool {
        matches!(self, Self::NetworkPolicies)
    }

    /// All sections, in document order.
    pub fn all() -> [Self; 4] {
        [
            Self::FilesystemPolicy,
            Self::Landlock,
            Self::Process,
            Self::NetworkPolicies,
        ]
    }
}

/// What a save will actually do.
///
/// Three outcomes, because there are three real cases. Collapsing them to a
/// boolean is what makes a UI claim a change is live when it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    /// The change is loaded by running sandboxes immediately.
    AppliesNow,
    /// Saved, but only a recreation picks it up.
    RequiresRecreate,
    /// Refused; the runtime rejects the change after the workload started.
    Rejected,
}

/// A policy as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Policy {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub scope: PolicyScope,
    /// The complete document. This is what the gateway receives and what diff
    /// and rollback operate on.
    pub spec: serde_json::Value,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A normalised network rule: the queryable projection of the document.
///
/// The document is authoritative. This table exists so a list or a bulk edit
/// does not have to parse JSON on every request. `policy::rules_from_document`
/// and `policy::document_from_rules` keep the two aligned, and a test asserts
/// the round trip.
///
/// `PartialEq` exists so a compiled rule set can be compared in a test without
/// spelling out fifteen fields; `Eq` is not derivable because nothing here
/// needs it beyond the `CompiledPolicy` that holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct NetworkRule {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub policy_id: Option<Uuid>,
    pub sandbox_id: Option<Uuid>,
    pub name: String,
    pub action: RuleAction,
    pub host: String,
    pub port: Option<i32>,
    pub protocol: RuleProtocol,
    pub methods: Vec<String>,
    /// The allowlist of executables. A script calling `curl` is not `curl`.
    pub binaries: Vec<String>,
    /// The conditions of the rule, since migration 0032 the authoritative
    /// form. An empty list means "no condition": a deny rule without matchers
    /// is the honest way to write default-deny. The flat fields above are kept
    /// for readers that predate the change and are written in step.
    #[serde(default)]
    pub matchers: Vec<Matcher>,
    pub enforcement: Enforcement,
    pub priority: i32,
    pub enabled: bool,
    pub version: i64,
}

/// Which conditions a matcher kind expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatcherKind {
    /// One exact hostname.
    Host,
    /// Every host ending in this suffix (`.example.com` covers `api.example.com`).
    HostSuffix,
    /// An IPv4 or IPv6 prefix (`10.0.0.0/8`), matched against literal IP hosts.
    Cidr,
    /// A port or a range (`443`, `8000-8100`).
    Port,
    /// One wire protocol.
    Protocol,
    /// One HTTP method.
    Method,
    /// One executable path.
    Binary,
    /// No condition. Written explicitly so a reader never has to know that an
    /// empty list means the same thing.
    Any,
}

impl MatcherKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::HostSuffix => "host_suffix",
            Self::Cidr => "cidr",
            Self::Port => "port",
            Self::Protocol => "protocol",
            Self::Method => "method",
            Self::Binary => "binary",
            Self::Any => "any",
        }
    }

    /// Whether this kind carries a value. `Any` is the only one that does not,
    /// and `{"kind":"any","value":"..."}` is a typo the constructor refuses.
    pub fn takes_value(self) -> bool {
        !matches!(self, Self::Any)
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "host" => Self::Host,
            "host_suffix" => Self::HostSuffix,
            "cidr" => Self::Cidr,
            "port" => Self::Port,
            "protocol" => Self::Protocol,
            "method" => Self::Method,
            "binary" => Self::Binary,
            "any" => Self::Any,
            _ => return None,
        })
    }
}

/// One condition of a network rule.
///
/// The JSON form is `{"kind":"host","value":"api.example.com"}`; `Any` is
/// `{"kind":"any"}`. The constructor validates the value against the kind, so a
/// malformed `cidr` is a `PolicyError` at construction, not a rule that
/// silently matches nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Matcher {
    pub kind: MatcherKind,
    /// The value the kind matches on. Absent only for `Any`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl Matcher {
    /// Build and validate one matcher.
    pub fn new(kind: MatcherKind, value: Option<String>) -> Result<Self, PolicyError> {
        match (kind, value.as_deref().map(str::trim)) {
            (MatcherKind::Any, None | Some("")) => {}
            (MatcherKind::Any, Some(_)) => {
                return Err(PolicyError::MatcherValue {
                    kind: "any".into(),
                    why: "the any kind takes no value".into(),
                })
            }
            (_, None | Some("")) => {
                return Err(PolicyError::MatcherValue {
                    kind: kind.as_str().to_string(),
                    why: "needs a non-empty value".into(),
                })
            }
            (MatcherKind::Host, Some(value)) => {
                if value.is_empty() || value.contains(' ') {
                    return Err(PolicyError::MatcherValue {
                        kind: "host".into(),
                        why: "a host is one name without spaces".into(),
                    });
                }
            }
            (MatcherKind::HostSuffix, Some(value)) => {
                // A suffix must be a dot-suffix or a full domain; `example.com`
                // is accepted and read as `.example.com` when matching, so the
                // value stays human-shaped.
                if !value.contains('.') || value.starts_with('.') && value.len() < 2 {
                    return Err(PolicyError::MatcherValue {
                        kind: "host_suffix".into(),
                        why: "a suffix is a domain such as .example.com".into(),
                    });
                }
            }
            (MatcherKind::Cidr, Some(value)) => {
                if !cidr_is_valid(value) {
                    return Err(PolicyError::MatcherValue {
                        kind: "cidr".into(),
                        why: "a CIDR prefix is address/length, such as 10.0.0.0/8".into(),
                    });
                }
            }
            (MatcherKind::Port, Some(value)) => {
                if port_range(value).is_none() {
                    return Err(PolicyError::MatcherValue {
                        kind: "port".into(),
                        why: "a port is a number or a range a-b, 1 to 65535".into(),
                    });
                }
            }
            (MatcherKind::Protocol, Some(value)) => {
                if RuleProtocol::parse(value).is_none() {
                    return Err(PolicyError::MatcherValue {
                        kind: "protocol".into(),
                        why: "one of rest, http, websocket, graphql, mcp, jsonrpc, tcp".into(),
                    });
                }
            }
            (MatcherKind::Method, Some(value)) => {
                if !value.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Err(PolicyError::MatcherValue {
                        kind: "method".into(),
                        why: "an HTTP method is letters, such as GET or POST".into(),
                    });
                }
            }
            (MatcherKind::Binary, Some(value)) => {
                if value.contains(' ') || value.contains('\0') {
                    return Err(PolicyError::MatcherValue {
                        kind: "binary".into(),
                        why: "an executable path is one word".into(),
                    });
                }
            }
        }

        Ok(Self { kind, value })
    }

    /// Whether one request side satisfies this matcher.
    ///
    /// The subject is the request flattened to the five things a rule may name.
    /// Evaluation is pure, so a rule is testable without a database or a
    /// runtime.
    pub fn matches(&self, subject: &MatchSubject<'_>) -> bool {
        let Some(value) = self.value.as_deref() else {
            return self.kind == MatcherKind::Any;
        };
        match self.kind {
            MatcherKind::Any => true,
            MatcherKind::Host => host_glob_matches(value, subject.host),
            MatcherKind::HostSuffix => host_ends_with_suffix(value, subject.host),
            MatcherKind::Cidr => cidr_contains(value, subject.host),
            MatcherKind::Port => {
                // A request port outside the rule vocabulary cannot be inside
                // a rule's range; comparing after a cast would make 70000
                // match 4464 and that is exactly the kind of lie a cast hides.
                if !(1..=65535).contains(&subject.port) {
                    return false;
                }
                let port = u16::try_from(subject.port).unwrap_or(0);
                port_range(value)
                    .map(|(low, high)| port >= low && port <= high)
                    .unwrap_or(false)
            }
            MatcherKind::Protocol => protocol_word_matches(value, subject.protocol),
            MatcherKind::Method => subject
                .method
                .map(|m| m.eq_ignore_ascii_case(value))
                .unwrap_or(false),
            MatcherKind::Binary => subject
                .binary
                .map(|b| b == value)
                .unwrap_or(false),
        }
    }
}

/// The request side of a match, flattened to what rules may name.
///
/// `binary` is what executed the request, when the caller knows it; a rule
/// naming a binary is untestable against an unknown executor, so it simply does
/// not match rather than matching on a guess.
#[derive(Debug, Clone, Copy)]
pub struct MatchSubject<'a> {
    pub host: &'a str,
    pub port: i32,
    pub protocol: Option<&'a str>,
    pub method: Option<&'a str>,
    pub binary: Option<&'a str>,
}

/// Whether `host` ends with `suffix`, where a bare domain counts as its own
/// `.suffix`.
///
/// `host_ends_with_suffix(".example.com", "api.example.com")` is true and
/// `host_ends_with_suffix(".example.com", "notexample.com")` is false, which is
/// the difference that makes the suffix useful.
fn host_ends_with_suffix(suffix: &str, host: &str) -> bool {
    let suffix = suffix.trim_start_matches('.');
    let dotted = format!(".{suffix}");
    let host = host.trim_end_matches('.');
    host == suffix || host.ends_with(&dotted)
}

/// Whether `value` is a well-formed CIDR prefix.
fn cidr_is_valid(value: &str) -> bool {
    cidr_prefix(value).is_some()
}

/// Parse `address/length` into the address and the prefix length.
fn cidr_prefix(value: &str) -> Option<(std::net::IpAddr, u8)> {
    let (address, length) = value.split_once('/')?;
    let length: u8 = length.trim().parse().ok()?;
    let address: std::net::IpAddr = address.trim().parse().ok()?;
    let max = match address {
        std::net::IpAddr::V4(_) => 32,
        std::net::IpAddr::V6(_) => 128,
    };
    if length > max {
        return None;
    }
    Some((address, length))
}

/// Whether `host` — which may be a name or a literal IP — falls inside the
/// prefix. A name cannot be inside a prefix; a literal IP can, and that is the
/// whole point.
fn cidr_contains(cidr: &str, host: &str) -> bool {
    let Some((base, length)) = cidr_prefix(cidr) else {
        return false;
    };
    let Ok(address) = host.trim().parse::<std::net::IpAddr>() else {
        return false;
    };
    match (base, address) {
        (std::net::IpAddr::V4(base), std::net::IpAddr::V4(address)) => {
            let base = u32::from(base);
            let address = u32::from(address);
            let shift = 32 - u32::from(length);
            (base >> shift) == (address >> shift)
        }
        (std::net::IpAddr::V6(base), std::net::IpAddr::V6(address)) => {
            let base = u128::from(base);
            let address = u128::from(address);
            let shift = 128 - u32::from(length);
            (base >> shift) == (address >> shift)
        }
        _ => false,
    }
}

/// Parse `443` or `8000-8100` into an inclusive range.
fn port_range(value: &str) -> Option<(u16, u16)> {
    let (low, high) = match value.split_once('-') {
        Some((low, high)) => (low.trim().parse().ok()?, high.trim().parse().ok()?),
        None => {
            let port: u16 = value.trim().parse().ok()?;
            (port, port)
        }
    };
    if low == 0 || high == 0 || high < low {
        return None;
    }
    Some((low, high))
}

/// One protocol word against the request's wire protocol.
///
/// The same tolerance `RuleProtocol` has: `http` covers https and the
/// application protocols that ride on it, `tcp` is the wildcard.
fn protocol_word_matches(value: &str, request: Option<&str>) -> bool {
    let Some(rule) = RuleProtocol::parse(value) else {
        return false;
    };
    let request = request.unwrap_or("tcp");
    match rule {
        RuleProtocol::Tcp => true,
        RuleProtocol::Rest | RuleProtocol::Http => {
            matches!(request, "http" | "https" | "rest" | "mcp" | "jsonrpc" | "graphql")
        }
        RuleProtocol::Websocket => matches!(request, "websocket" | "ws" | "wss"),
        RuleProtocol::Graphql => request == "graphql",
        RuleProtocol::Mcp => request == "mcp",
        RuleProtocol::Jsonrpc => matches!(request, "jsonrpc" | "mcp"),
    }
}

/// The matcher form of a rule that arrives without one.
///
/// Migration 0032 filled the column for every existing row; this covers a
/// caller that sends the old flat shape and lets one constructor own both.
/// `tcp` disappears because it is the wildcard protocol and carries no
/// condition.
pub fn matchers_from_flat(
    host: &str,
    port: Option<i32>,
    protocol: RuleProtocol,
    methods: &[String],
    binaries: &[String],
) -> Vec<Matcher> {
    let mut matchers = Vec::new();
    if !host.is_empty() && host != "*" {
        matchers.push(Matcher {
            kind: MatcherKind::Host,
            value: Some(host.to_string()),
        });
    }
    if let Some(port) = port {
        matchers.push(Matcher {
            kind: MatcherKind::Port,
            value: Some(port.to_string()),
        });
    }
    if protocol != RuleProtocol::Tcp {
        matchers.push(Matcher {
            kind: MatcherKind::Protocol,
            value: Some(protocol.as_str().to_string()),
        });
    }
    for method in methods {
        matchers.push(Matcher {
            kind: MatcherKind::Method,
            value: Some(method.clone()),
        });
    }
    for binary in binaries {
        matchers.push(Matcher {
            kind: MatcherKind::Binary,
            value: Some(binary.clone()),
        });
    }
    matchers
}

/// The flat fields of a rule that arrives without matchers, kept so a legacy
/// caller's rule still means what it meant.
///
/// Rules are normalised on the way in: a rule with matchers is stored as sent,
/// a rule without matchers gets them derived from the flat fields. There is no
/// third case, so the decider never has to guess which form it is reading.
pub fn normalise_matchers(rule: &mut NetworkRule) {
    if rule.matchers.is_empty() {
        rule.matchers = matchers_from_flat(
            &rule.host,
            rule.port,
            rule.protocol,
            &rule.methods,
            &rule.binaries,
        );
    }
}

/// Derive the flat fields readers that predate 0032 still show.
///
/// One host, one port, one protocol word, all methods, all binaries — exactly
/// what one matcher list can say. A rule the flat form cannot express (a CIDR
/// or a port range) still has its matchers, and the decider reads those; the
/// flat host becomes `*` so the legacy reader sees "unconditional" rather than
/// a host the rule does not name.
pub fn flat_from_matchers(matchers: &[Matcher]) -> FlatRule {
    let mut flat = FlatRule {
        host: String::new(),
        port: None,
        protocol: RuleProtocol::Tcp,
        methods: Vec::new(),
        binaries: Vec::new(),
    };
    let mut named_host = false;
    for matcher in matchers {
        let Some(value) = matcher.value.as_deref() else {
            continue;
        };
        match matcher.kind {
            MatcherKind::Any => {}
            MatcherKind::Host => {
                flat.host = value.to_string();
                named_host = true;
            }
            MatcherKind::HostSuffix => {
                if !named_host {
                    flat.host = value.to_string();
                }
                named_host = true;
            }
            MatcherKind::Cidr => {
                if !named_host {
                    // The legacy column is NOT NULL and has no IP meaning; a
                    // bare star is the honest "see the matchers" answer.
                    flat.host = "*".to_string();
                }
                named_host = true;
            }
            MatcherKind::Port => {
                if flat.port.is_none() {
                    flat.port = port_range(value).and_then(|(low, high)| {
                        // A range cannot be one column; the matchers stay
                        // exact and the legacy column shows nothing.
                        (low == high).then(|| i32::from(low))
                    });
                }
            }
            MatcherKind::Protocol => {
                if let Some(rule) = RuleProtocol::parse(value) {
                    flat.protocol = rule;
                }
            }
            MatcherKind::Method => flat.methods.push(value.to_string()),
            MatcherKind::Binary => flat.binaries.push(value.to_string()),
        }
    }
    if flat.host.is_empty() {
        flat.host = "*".to_string();
    }
    flat
}

/// The flat shape of a rule, for readers that predate the matcher form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatRule {
    pub host: String,
    pub port: Option<i32>,
    pub protocol: RuleProtocol,
    pub methods: Vec<String>,
    pub binaries: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Allow,
    Deny,
}

impl RuleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleProtocol {
    Rest,
    Http,
    Websocket,
    Graphql,
    Mcp,
    Jsonrpc,
    Tcp,
}

impl RuleProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Http => "http",
            Self::Websocket => "websocket",
            Self::Graphql => "graphql",
            Self::Mcp => "mcp",
            Self::Jsonrpc => "jsonrpc",
            Self::Tcp => "tcp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "rest" => Self::Rest,
            "http" => Self::Http,
            "websocket" => Self::Websocket,
            "graphql" => Self::Graphql,
            "mcp" => Self::Mcp,
            "jsonrpc" => Self::Jsonrpc,
            "tcp" => Self::Tcp,
            _ => return None,
        })
    }
}

/// Whether a rule blocks or merely records.
///
/// `Audit` is how a policy is developed without breaking the workload: the
/// attempt is logged and allowed. The console shows it as a warning, not a wall,
/// because an audit-mode block is not a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    Enforced,
    Audit,
}

impl Enforcement {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforced => "enforced",
            Self::Audit => "audit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "enforced" => Some(Self::Enforced),
            "audit" => Some(Self::Audit),
            _ => None,
        }
    }

    /// Whether this rule actually prevents traffic.
    pub fn blocks(self) -> bool {
        matches!(self, Self::Enforced)
    }
}

/// Which firewall a policy document describes, as the console groups it.
///
/// Two kinds, because the document says exactly that much. A document that
/// carries only `network_policies` is about egress and nothing else; a document
/// that also carries `filesystem_policy`, `landlock` or `process` is the whole
/// sandbox fence, which is what people mean by an OpenShell policy. DNS rules
/// are rows in their own table, not document sections, so they are not a kind
/// here — a policy that carries a DNS rule is still a network policy with a
/// separate companion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    Openshell,
    Network,
}

impl PolicyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openshell => "openshell",
            Self::Network => "network",
        }
    }
}

/// The sections a document actually carries, in document order.
///
/// Reading the keys instead of trusting a label is what keeps the badge true: a
/// document edited to add a filesystem section becomes an OpenShell policy
/// without anyone updating a column.
pub fn document_sections(spec: &serde_json::Value) -> Vec<&'static str> {
    let Some(object) = spec.as_object() else {
        return Vec::new();
    };
    PolicySection::all()
        .into_iter()
        .filter(|section| object.contains_key(section.as_str()))
        .map(|section| section.as_str())
        .collect()
}

/// Split the sections a document carries into hot and cold (ADR 0008).
///
/// Hot sections take effect on a running sandbox; cold ones are fixed when the
/// workload starts, so a change needs a recreate. Order follows the document.
pub fn hot_and_cold(spec: &serde_json::Value) -> (Vec<&'static str>, Vec<&'static str>) {
    let mut hot = Vec::new();
    let mut cold = Vec::new();
    for name in document_sections(spec) {
        let hot_reloadable = PolicySection::all()
            .into_iter()
            .any(|section| section.as_str() == name && section.is_hot_reloadable());
        if hot_reloadable {
            hot.push(name);
        } else {
            cold.push(name);
        }
    }
    (hot, cold)
}

/// Classify one document.
pub fn classify(spec: &serde_json::Value) -> PolicyKind {
    let sections = document_sections(spec);
    let startup = sections
        .iter()
        .any(|section| *section != PolicySection::NetworkPolicies.as_str());
    if startup {
        PolicyKind::Openshell
    } else {
        PolicyKind::Network
    }
}

/// Validate a policy document and report which sections it touches.
///
/// Returns the sections present so the caller can decide whether a save is a
/// live change or needs a recreation. Every section's known fields are checked
/// against their vocabulary, so a typo (`access: "alow"`) is a refusal with the
/// reason and not a document the runtime silently ignores. Unknown fields are
/// left alone on purpose: the runtime schema gains fields before this check
/// does, and a validation that rejected them would block a working runtime.
pub fn validate_document(spec: &serde_json::Value) -> Result<Vec<PolicySection>, PolicyError> {
    let object = spec
        .as_object()
        .ok_or_else(|| PolicyError::NotAnObject)?;

    let mut sections = Vec::new();
    for section in PolicySection::all() {
        if let Some(value) = object.get(section.as_str()) {
            validate_section(section, value)?;
            sections.push(section);
        }
    }

    if sections.is_empty() {
        return Err(PolicyError::Empty);
    }

    Ok(sections)
}

/// Validate one section against its schema.
///
/// Each section is a small vocabulary, and this is the one place the words are
/// checked. The shape is what the console's form editor renders from
/// [`policy_schema`], so a field the editor offers is a field this validates.
fn validate_section(section: PolicySection, value: &serde_json::Value) -> Result<(), PolicyError> {
    let object = value.as_object().ok_or_else(|| {
        PolicyError::SectionNotAnObject {
            section: section.as_str().to_string(),
        }
    })?;

    match section {
        PolicySection::NetworkPolicies => validate_network_policies(object),
        PolicySection::FilesystemPolicy => validate_paths(object, "paths", PATH_ACCESS),
        PolicySection::Landlock => validate_paths(object, "rules", LANDLOCK_ACCESS),
        PolicySection::Process => validate_process(object),
    }
}

/// The access words the filesystem section understands.
const PATH_ACCESS: &[&str] = &["read", "write", "read_write", "exec", "none"];

/// The access words the Landlock section understands.
const LANDLOCK_ACCESS: &[&str] = &["read", "write", "exec"];

/// `filesystem_policy` and `landlock` share one shape: a list of path grants.
fn validate_paths(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    vocabulary: &[&str],
) -> Result<(), PolicyError> {
    let section = if key == "paths" {
        "filesystem_policy"
    } else {
        "landlock"
    };
    let Some(entries) = object.get(key) else {
        // An empty section is a valid statement: nothing granted.
        return Ok(());
    };
    let entries = entries.as_array().ok_or_else(|| PolicyError::Section {
        section: section.to_string(),
        why: format!("{key} must be a list of entries"),
    })?;

    for entry in entries {
        let entry = entry.as_object().ok_or_else(|| PolicyError::Section {
            section: section.to_string(),
            why: format!("every {key} entry is an object with path and access"),
        })?;
        let path = entry
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if path.is_empty() || !path.starts_with('/') {
            return Err(PolicyError::Section {
                section: section.to_string(),
                why: format!("every {key} entry names an absolute path, not {path:?}"),
            });
        }
        let access = match entry.get("access") {
            Some(serde_json::Value::Array(words)) => words
                .iter()
                .filter_map(|w| w.as_str())
                .collect::<Vec<_>>(),
            Some(serde_json::Value::String(word)) => vec![word.as_str()],
            Some(_) => {
                return Err(PolicyError::Section {
                    section: section.to_string(),
                    why: "access is one word or a list of words".to_string(),
                })
            }
            None => return Err(PolicyError::Section {
                section: section.to_string(),
                why: "every entry says what access it grants".to_string(),
            }),
        };
        for word in &access {
            if !vocabulary.contains(word) {
                return Err(PolicyError::Section {
                    section: section.to_string(),
                    why: format!(
                        "access is one of {}; {word:?} is not",
                        vocabulary.join(", ")
                    ),
                });
            }
        }
    }
    Ok(())
}

/// `process`: the user, the binaries, the environment, the process budget.
fn validate_process(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), PolicyError> {
    if let Some(user) = object.get("run_as_user") {
        let user = user.as_str().ok_or_else(|| PolicyError::Section {
            section: "process".to_string(),
            why: "run_as_user is a string".to_string(),
        })?;
        if user.is_empty() || user.contains(' ') {
            return Err(PolicyError::Section {
                section: "process".to_string(),
                why: "run_as_user is one user name".to_string(),
            });
        }
    }
    if let Some(binaries) = object.get("binaries") {
        let binaries = binaries.as_array().ok_or_else(|| PolicyError::Section {
            section: "process".to_string(),
            why: "binaries is a list of executable paths".to_string(),
        })?;
        for binary in binaries {
            let path = binary
                .as_str()
                .or_else(|| binary.get("path").and_then(|v| v.as_str()))
                .unwrap_or_default();
            if path.is_empty() || !path.starts_with('/') {
                return Err(PolicyError::Section {
                    section: "process".to_string(),
                    why: format!("a binary is an absolute path, not {path:?}"),
                });
            }
        }
    }
    if let Some(allow) = object.get("env_allow") {
        let allow = allow.as_array().ok_or_else(|| PolicyError::Section {
            section: "process".to_string(),
            why: "env_allow is a list of variable names".to_string(),
        })?;
        for name in allow {
            let name = name.as_str().unwrap_or_default();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(PolicyError::Section {
                    section: "process".to_string(),
                    why: format!("an environment variable is letters, digits, _, not {name:?}"),
                });
            }
        }
    }
    if let Some(max) = object.get("max_processes") {
        let max = max.as_i64().ok_or_else(|| PolicyError::Section {
            section: "process".to_string(),
            why: "max_processes is a whole number".to_string(),
        })?;
        if max < 1 || max > 4096 {
            return Err(PolicyError::Section {
                section: "process".to_string(),
                why: "max_processes is between 1 and 4096".to_string(),
            });
        }
    }
    Ok(())
}

/// `network_policies`: named policies, each with endpoints that carry the rule
/// vocabulary the projection reads.
fn validate_network_policies(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), PolicyError> {
    for (name, policy) in object {
        let policy = policy.as_object().ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: format!("the policy {name:?} is an object with endpoints"),
        })?;
        if let Some(enforcement) = policy.get("enforcement") {
            let word = enforcement.as_str().unwrap_or_default();
            if Enforcement::parse(word).is_none() {
                return Err(PolicyError::Section {
                    section: "network_policies".to_string(),
                    why: format!("enforcement is enforced or audit, not {word:?}"),
                });
            }
        }
        let endpoints = policy.get("endpoints").ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: format!("the policy {name:?} says which endpoints it covers"),
        })?;
        let endpoints = endpoints.as_array().ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: format!("the endpoints of {name:?} are a list"),
        })?;
        for endpoint in endpoints {
            validate_endpoint(endpoint)?;
        }
    }
    Ok(())
}

/// One endpoint of a named network policy.
fn validate_endpoint(endpoint: &serde_json::Value) -> Result<(), PolicyError> {
    let endpoint = endpoint.as_object().ok_or_else(|| PolicyError::Section {
        section: "network_policies".to_string(),
        why: "every endpoint is an object".to_string(),
    })?;
    let host = endpoint
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if host.is_empty() || host.contains(' ') {
        return Err(PolicyError::Section {
            section: "network_policies".to_string(),
            why: format!("every endpoint names a host, not {host:?}"),
        });
    }
    if let Some(port) = endpoint.get("port") {
        let port = port.as_i64().ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: "a port is a whole number".to_string(),
        })?;
        if !(1..=65535).contains(&port) {
            return Err(PolicyError::Section {
                section: "network_policies".to_string(),
                why: "a port is between 1 and 65535".to_string(),
            });
        }
    }
    if let Some(protocol) = endpoint.get("protocol") {
        let word = protocol.as_str().unwrap_or_default();
        if RuleProtocol::parse(word).is_none() {
            return Err(PolicyError::Section {
                section: "network_policies".to_string(),
                why: format!(
                    "a protocol is one of rest, http, websocket, graphql, mcp, jsonrpc, tcp; not {word:?}"
                ),
            });
        }
    }
    if let Some(access) = endpoint.get("access") {
        let word = access.as_str().unwrap_or_default();
        if RuleAction::parse(word).is_none() {
            return Err(PolicyError::Section {
                section: "network_policies".to_string(),
                why: format!("access is allow or deny, not {word:?}"),
            });
        }
    }
    if let Some(methods) = endpoint.get("methods") {
        let methods = methods.as_array().ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: "methods is a list of HTTP methods".to_string(),
        })?;
        for method in methods {
            let word = method.as_str().unwrap_or_default();
            if word.is_empty() || !word.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(PolicyError::Section {
                    section: "network_policies".to_string(),
                    why: format!("an HTTP method is letters, not {word:?}"),
                });
            }
        }
    }
    if let Some(binaries) = endpoint.get("binaries") {
        let binaries = binaries.as_array().ok_or_else(|| PolicyError::Section {
            section: "network_policies".to_string(),
            why: "binaries is a list of executable paths".to_string(),
        })?;
        for binary in binaries {
            let path = binary
                .as_str()
                .or_else(|| binary.get("path").and_then(|v| v.as_str()))
                .unwrap_or_default();
            if path.is_empty() || !path.starts_with('/') {
                return Err(PolicyError::Section {
                    section: "network_policies".to_string(),
                    why: format!("a binary is an absolute path, not {path:?}"),
                });
            }
        }
    }
    Ok(())
}

/// The document schema, for the console's editor.
///
/// This is data, not prose: the editor renders its fields from it, so a field
/// the schema knows and one the editor offers cannot drift apart. Every entry
/// carries the vocabulary the validation accepts, which is why the console can
/// validate before it saves.
pub fn policy_schema() -> serde_json::Value {
    serde_json::json!({
        "sections": [
            {
                "key": "filesystem_policy",
                "title": "Filesystem",
                "hot_reloadable": false,
                "description": "Which paths the sandbox may read, write, or execute.",
                "list_key": "paths",
                "fields": [
                    { "name": "path", "type": "string", "required": true, "example": "/workspace",
                      "description": "An absolute path inside the sandbox." },
                    { "name": "access", "type": "enum", "required": true,
                      "values": PATH_ACCESS,
                      "description": "What may happen on this path." },
                ],
            },
            {
                "key": "landlock",
                "title": "Landlock",
                "hot_reloadable": false,
                "description": "Kernel-enforced path grants; the same shape as the filesystem section.",
                "list_key": "rules",
                "fields": [
                    { "name": "path", "type": "string", "required": true, "example": "/home/nestbox",
                      "description": "An absolute path inside the sandbox." },
                    { "name": "access", "type": "enum-list", "required": true,
                      "values": LANDLOCK_ACCESS,
                      "description": "What may happen on this path." },
                ],
            },
            {
                "key": "process",
                "title": "Process",
                "hot_reloadable": false,
                "description": "Who runs, which executables may start, and which environment variables survive.",
                "fields": [
                    { "name": "run_as_user", "type": "string", "required": false, "example": "sandbox" },
                    { "name": "binaries", "type": "path-list", "required": false,
                      "example": "/usr/local/bin/agent" },
                    { "name": "env_allow", "type": "string-list", "required": false, "example": "PATH" },
                    { "name": "max_processes", "type": "integer", "required": false, "min": 1, "max": 4096 },
                ],
            },
            {
                "key": "network_policies",
                "title": "Network",
                "hot_reloadable": true,
                "description": "Named policies, each with the endpoints it covers. Changes reload on running sandboxes.",
                "fields": [
                    { "name": "enforcement", "type": "enum", "required": false,
                      "values": ["enforced", "audit"],
                      "description": "audit records and allows; enforced blocks." },
                ],
                "endpoint_fields": [
                    { "name": "host", "type": "string", "required": true, "example": "api.example.com" },
                    { "name": "port", "type": "integer", "required": false, "min": 1, "max": 65535 },
                    { "name": "protocol", "type": "enum", "required": false,
                      "values": ["rest", "http", "websocket", "graphql", "mcp", "jsonrpc", "tcp"] },
                    { "name": "access", "type": "enum", "required": false, "values": ["allow", "deny"] },
                    { "name": "methods", "type": "string-list", "required": false, "example": "GET" },
                    { "name": "binaries", "type": "path-list", "required": false },
                ],
            },
        ],
    })
}

/// Decide what a save will do, given the sections it changes and whether the
/// target sandboxes are already running.
///
/// This is the function that makes ADR 0008 true in code rather than in prose.
pub fn apply_outcome(changed: &[PolicySection], targets_running: bool) -> ApplyOutcome {
    if !targets_running {
        // Nothing is running, so the startup sections are still settable.
        return ApplyOutcome::AppliesNow;
    }
    if changed.iter().all(|s| s.is_hot_reloadable()) {
        ApplyOutcome::AppliesNow
    } else {
        // A startup section on a running sandbox cannot be applied. The runtime
        // rejects it, so the API reports "recreate" rather than pretending.
        ApplyOutcome::RequiresRecreate
    }
}

/// Errors a policy document can produce.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("policy document must be a JSON object")]
    NotAnObject,
    #[error("policy document has no recognised section")]
    Empty,
    #[error("matcher {kind} is not valid: {why}")]
    MatcherValue {
        kind: String,
        why: String,
    },
    #[error("section {section} is not valid: {why}")]
    Section {
        section: String,
        why: String,
    },
    #[error("document section {section} must be a JSON object")]
    SectionNotAnObject {
        section: String,
    },
}

impl PolicyError {
    /// A stable code for the API envelope. Clients branch on this, never on the
    /// human-readable message (ADR 0015).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAnObject => "policy.not_an_object",
            Self::Empty => "policy.empty",
            Self::MatcherValue { .. } => "policy.matcher_invalid",
            Self::Section { .. } => "policy.section_invalid",
            Self::SectionNotAnObject { .. } => "policy.section_not_an_object",
        }
    }
}

/// Project the `network_policies` section into flat rules.
///
/// The document is authoritative; this is a read-side projection. Keeping the
/// direction one-way means the projection can be rebuilt at any time and can
/// never disagree with the document it came from.
pub fn rules_from_document(spec: &serde_json::Value) -> Vec<NetworkRuleSeed> {
    let mut out = Vec::new();
    let Some(policies) = spec.get("network_policies").and_then(|v| v.as_object()) else {
        return out;
    };

    for (rule_name, policy) in policies {
        let Some(endpoints) = policy.get("endpoints").and_then(|v| v.as_array()) else {
            continue;
        };
        for endpoint in endpoints {
            let host = endpoint
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if host.is_empty() {
                continue;
            }
            let port = endpoint
                .get("port")
                .and_then(|v| v.as_i64())
                .and_then(|p| i32::try_from(p).ok());
            let protocol = endpoint
                .get("protocol")
                .and_then(|v| v.as_str())
                .and_then(RuleProtocol::parse)
                .unwrap_or(RuleProtocol::Rest);
            let action = match endpoint.get("access").and_then(|v| v.as_str()) {
                Some("deny") => RuleAction::Deny,
                _ => RuleAction::Allow,
            };
            let enforcement = endpoint
                .get("enforcement")
                .and_then(|v| v.as_str())
                .and_then(Enforcement::parse)
                .unwrap_or(Enforcement::Enforced);
            let methods: Vec<String> = endpoint
                .get("methods")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let binaries: Vec<String> = endpoint
                .get("binaries")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|b| b.get("path").and_then(|p| p.as_str()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();

            let matchers =
                matchers_from_flat(&host, port, protocol, &methods, &binaries);

            out.push(NetworkRuleSeed {
                matchers,
                name: rule_name.clone(),
                action,
                host,
                port,
                protocol,
                methods,
                binaries,
                enforcement,
            });
        }
    }
    out
}

/// A DNS rule: which names this sandbox may resolve.
///
/// `pattern` is an exact name or a `.suffix`; the same matching rule the
/// console shows. Actions are the rule vocabulary's two answers, so a
/// reader never has to learn a second word for "block".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct DnsRule {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub policy_id: Option<Uuid>,
    pub sandbox_id: Option<Uuid>,
    pub action: RuleAction,
    /// An exact name or a `.suffix` such as `.example.net`.
    pub pattern: String,
    pub priority: i32,
    pub enabled: bool,
    pub version: i64,
}

/// How deeply one sandbox is observed on the way out.
///
/// Four modes, because there are four real answers: no proxy at all
/// (`direct`), the proxy sees the request and nothing more (`forward_proxy`),
/// the proxy terminates TLS and reads the decrypted request (`transparent`), or
/// both per port (`hybrid`).
///
/// A mode is a per-sandbox row, not a global switch, so a sandbox that must not
/// have its TLS read is not read, and a sandbox that must not be proxied is not
/// proxied. The mode is the *observation* answer; where traffic leaves the node
/// is a separate choice (`sandboxes.proxy_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EgressMode {
    Direct,
    ForwardProxy,
    Transparent,
    Hybrid,
    /// The proxy only tunnels and blocks nothing: every connection is allowed
    /// and recorded, and the OpenShell supervisor runs in audit mode. A
    /// deliberate, loudly warned-about state (ADR 0053).
    OffAllowAll,
}

impl EgressMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::ForwardProxy => "forward_proxy",
            Self::Transparent => "transparent",
            Self::Hybrid => "hybrid",
            Self::OffAllowAll => "off_allow_all",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "direct" => Self::Direct,
            "forward_proxy" => Self::ForwardProxy,
            "transparent" => Self::Transparent,
            "hybrid" => Self::Hybrid,
            "off_allow_all" => Self::OffAllowAll,
            _ => return None,
        })
    }

    /// True when this mode lets every connection through and only records it.
    pub fn allows_everything(self) -> bool {
        matches!(self, Self::OffAllowAll)
    }

    /// True for the modes an end user may pick. `direct` changes the network and
    /// the exit proxy, so it stays an administrator's choice that rebuilds.
    pub fn user_selectable(self) -> bool {
        !matches!(self, Self::Direct)
    }

    /// Whether this mode puts the node's egress proxy in the path at all.
    pub fn uses_proxy(self) -> bool {
        !matches!(self, Self::Direct)
    }

    /// Whether this mode may read the contents of a TLS connection, and
    /// therefore needs the sandbox to trust the node's CA.
    pub fn inspects_tls(self) -> bool {
        matches!(self, Self::Transparent | Self::Hybrid)
    }
}

/// One sandbox's egress mode.
///
/// `trust_ca_ref` is a locator for the sandbox-scoped trust CA (ADR 0005): a
/// mode that reads TLS needs the sandbox to trust the node's CA, and the key
/// material never travels through this row.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EgressModeSetting {
    pub sandbox_id: Uuid,
    pub mode: EgressMode,
    pub trust_ca_ref: Option<String>,
    pub enabled: bool,
    pub version: i64,
    /// When the mode last changed.
    #[serde(default)]
    pub changed_at: Option<DateTime<Utc>>,
    /// When it returns to `revert_to` by itself, if a timer was set.
    #[serde(default)]
    pub revert_at: Option<DateTime<Utc>>,
    /// The mode it returns to.
    #[serde(default)]
    pub revert_to: Option<String>,
    /// True when this sandbox was created before the CA was mounted for every
    /// mode: one recreate is needed before TLS can be opened.
    #[serde(default)]
    pub needs_recreate: bool,
}

/// A preset in the template library.
///
/// Templates are data (ADR 0020): a preset is a document with a name, so an
/// operator can edit one and the change needs no deploy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyTemplate {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub description: String,
    /// The document the template produces. Validated like every save.
    pub spec: serde_json::Value,
    pub version: i64,
}

/// Which policy is assigned to one sandbox, and what of it has been applied.
///
/// `desired_version` is what the control plane wants; `applied_version` is what
/// the sandbox has loaded. The gap between the two is the drift the console
/// shows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PolicyAssignment {
    pub sandbox_id: Uuid,
    pub policy_id: Uuid,
    pub policy_name: String,
    pub policy_scope: PolicyScope,
    pub desired_version: i64,
    pub applied_version: Option<i64>,
    pub applied_at: Option<DateTime<Utc>>,
}

/// A rule extracted from a document, before it has an identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRuleSeed {
    pub name: String,
    pub action: RuleAction,
    pub host: String,
    pub port: Option<i32>,
    pub protocol: RuleProtocol,
    pub methods: Vec<String>,
    pub binaries: Vec<String>,
    pub enforcement: Enforcement,
    /// The same conditions in matcher form, built with `matchers_from_flat`.
    pub matchers: Vec<Matcher>,
}

/// Priorities of rules that come from a policy document. They sit behind every
/// stored rule (whose default is 100), so an operator's explicit rule always wins
/// over the document it was made next to, and a deny in the document is read
/// before an allow in it.
pub const DOCUMENT_DENY_PRIORITY: i32 = 500;
pub const DOCUMENT_ALLOW_PRIORITY: i32 = 1000;

/// The rules a policy document grants, as the decider reads them.
///
/// The document's `network_policies` section used to be inert for egress: an
/// assignment wrote a row and the decider read only `network_rules`, so a sandbox
/// with the "safe defaults" policy could reach nothing it was meant to. The rules
/// are compiled here, at decision time, from the current document, so editing a
/// policy changes the answer within the proxy's cache TTL with nothing to sync.
///
/// They carry the nil id: they are not rows, and a decision that cites one records
/// no rule id rather than an id that points at nothing.
pub fn rules_from_policy(policy: &Policy) -> Vec<NetworkRule> {
    rules_from_document(&policy.spec)
        .into_iter()
        .enumerate()
        .map(|(index, seed)| NetworkRule {
            id: Uuid::nil(),
            environment_id: policy.environment_id,
            policy_id: Some(policy.id),
            sandbox_id: None,
            name: format!("{}: {}", policy.name, seed.name),
            priority: match seed.action {
                RuleAction::Deny => DOCUMENT_DENY_PRIORITY,
                RuleAction::Allow => DOCUMENT_ALLOW_PRIORITY,
            } + i32::try_from(index).unwrap_or(0),
            action: seed.action,
            host: seed.host,
            port: seed.port,
            protocol: seed.protocol,
            methods: seed.methods,
            binaries: seed.binaries,
            matchers: seed.matchers,
            enforcement: seed.enforcement,
            enabled: true,
            version: policy.version,
        })
        .collect()
}

/// The stored rules that apply to one sandbox.
///
/// A rule on a sandbox applies to that sandbox. A rule with neither a sandbox nor
/// a policy applies to everyone. A rule on a policy applies only to the sandboxes
/// that carry that policy (or to all of them while it is the global override); the
/// decider used to treat it as applying to everyone, which made "add a rule to
/// policy A" quietly open policy B's sandboxes.
pub fn rules_for_sandbox_view(
    rules: Vec<NetworkRule>,
    sandbox_id: Uuid,
    governing_policy: Option<Uuid>,
) -> Vec<NetworkRule> {
    rules
        .into_iter()
        .filter(|rule| match (rule.sandbox_id, rule.policy_id) {
            (Some(target), _) => target == sandbox_id,
            (None, None) => true,
            (None, Some(policy)) => Some(policy) == governing_policy,
        })
        .collect()
}

/// Whether a DNS pattern covers a host: an exact name, or `.suffix` for the domain
/// and everything under it.
pub fn dns_pattern_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if pattern.is_empty() || host.is_empty() {
        return false;
    }
    match pattern.strip_prefix('.') {
        Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
        None => host == pattern,
    }
}

/// The DNS rule that decides for `host`, by ascending priority, among the rules
/// that apply to this sandbox (its own, its governing policy's, and unscoped ones).
pub fn dns_rule_for<'a>(
    rules: &'a [DnsRule],
    sandbox_id: Uuid,
    governing_policy: Option<Uuid>,
    host: &str,
) -> Option<&'a DnsRule> {
    let mut candidates: Vec<&DnsRule> = rules
        .iter()
        .filter(|rule| rule.enabled)
        .filter(|rule| match (rule.sandbox_id, rule.policy_id) {
            (Some(target), _) => target == sandbox_id,
            (None, None) => true,
            (None, Some(policy)) => Some(policy) == governing_policy,
        })
        .filter(|rule| dns_pattern_matches(&rule.pattern, host))
        .collect();
    candidates.sort_by_key(|rule| rule.priority);
    candidates.into_iter().next()
}

/// Host and port of an endpoint URL: `http://10.0.0.5:20128/v1` -> (`10.0.0.5`, 20128).
/// The scheme decides a missing port. `None` for something that is not a URL.
pub fn endpoint_host_port(endpoint: &str) -> Option<(String, i32)> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            (host, port.parse::<i32>().ok()?)
        }
        _ => (authority, if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 }),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

/// The inference endpoints an operator configured, as allow rules.
///
/// A provider profile *is* the statement "agents may talk to this model endpoint",
/// so needing a second, hand-written rule for the same host and port was a trap:
/// the agent was told to use the endpoint and the proxy then refused it. A
/// reachable profile whose host is a real address or name (not a bare container
/// name on the sandbox network, which bypasses the proxy) becomes an allow rule
/// behind every stored rule. A deny rule still wins over it.
pub fn rules_from_providers(profiles: &[crate::domain::provider::ProviderProfile]) -> Vec<NetworkRule> {
    profiles
        .iter()
        .filter(|profile| profile.status == crate::domain::provider::ProviderStatus::Reachable)
        .filter_map(|profile| {
            let (host, port) = endpoint_host_port(&profile.endpoint)?;
            if !host.contains('.') && !host.contains(':') {
                return None;
            }
            Some(NetworkRule {
                id: Uuid::nil(),
                environment_id: profile.environment_id,
                policy_id: None,
                sandbox_id: None,
                name: format!("Inference provider: {}", profile.name),
                action: RuleAction::Allow,
                host: host.clone(),
                port: Some(port),
                protocol: RuleProtocol::Tcp,
                methods: vec![],
                binaries: vec![],
                matchers: vec![
                    Matcher { kind: MatcherKind::Host, value: Some(host) },
                    Matcher { kind: MatcherKind::Port, value: Some(port.to_string()) },
                ],
                enforcement: Enforcement::Enforced,
                priority: DOCUMENT_ALLOW_PRIORITY - 100,
                enabled: true,
                version: profile.version,
            })
        })
        .collect()
}

/// The MCP servers attached to a sandbox, as allow rules.
///
/// Attaching a server is the statement "this sandbox may use it", and an HTTP
/// server is reached through the proxy, so without a rule the attachment would be
/// refused by the very control plane that made it. Only HTTP/SSE servers with an
/// endpoint produce a rule (a stdio server never crosses the proxy), and only for
/// that sandbox.
pub fn rules_from_mcp(
    servers: &[crate::domain::mcp::McpServer],
    sandbox_id: Uuid,
    environment_id: Uuid,
) -> Vec<NetworkRule> {
    servers
        .iter()
        .filter(|server| server.enabled)
        .filter_map(|server| {
            let (host, port) = endpoint_host_port(server.endpoint.as_deref()?)?;
            Some(NetworkRule {
                id: Uuid::nil(),
                environment_id,
                policy_id: None,
                sandbox_id: Some(sandbox_id),
                name: format!("MCP server: {}", server.name),
                action: RuleAction::Allow,
                host: host.clone(),
                port: Some(port),
                protocol: RuleProtocol::Mcp,
                methods: vec![],
                binaries: vec![],
                matchers: vec![
                    Matcher { kind: MatcherKind::Host, value: Some(host) },
                    Matcher { kind: MatcherKind::Port, value: Some(port.to_string()) },
                ],
                enforcement: Enforcement::Enforced,
                priority: DOCUMENT_ALLOW_PRIORITY - 100,
                enabled: true,
                version: 1,
            })
        })
        .collect()
}

/// The allow rule that lets one sandbox reach the accounting proxy.
///
/// Model traffic that is routed through the proxy (`route_through_accounting`)
/// has to be allowed to reach it, or the agent would be cut off by the control
/// plane that sent it there. The rule names the proxy's host and port and
/// nothing else; what the proxy forwards upstream is the proxy's own business
/// and is recorded there. Empty when the URL has no host.
pub fn rules_for_accounting_proxy(
    proxy_url: &str,
    sandbox_id: Uuid,
    environment_id: Uuid,
) -> Vec<NetworkRule> {
    let Some((host, port)) = endpoint_host_port(proxy_url) else {
        return Vec::new();
    };
    vec![NetworkRule {
        id: Uuid::nil(),
        environment_id,
        policy_id: None,
        sandbox_id: Some(sandbox_id),
        name: "Accounting proxy".to_string(),
        action: RuleAction::Allow,
        host: host.clone(),
        port: Some(port),
        protocol: RuleProtocol::Tcp,
        methods: vec![],
        binaries: vec![],
        matchers: vec![
            Matcher { kind: MatcherKind::Host, value: Some(host) },
            Matcher { kind: MatcherKind::Port, value: Some(port.to_string()) },
        ],
        enforcement: Enforcement::Enforced,
        priority: DOCUMENT_ALLOW_PRIORITY - 100,
        enabled: true,
        version: 1,
    }]
}

/// The allow rule that lets a gateway sandbox reach the egress proxy.
///
/// On an OpenShell gateway every destination must be named in the policy,
/// including our own proxy. The rule grants the *hop* to the proxy and nothing
/// else: what the agent may actually reach is still decided per request by the
/// proxy (ADR 0013), so it cannot widen access and it cannot be skipped.
///
/// It is deliberately `tcp`: a forward proxy speaks `CONNECT`, so there is no
/// request inside the tunnel for OpenShell to parse at L7.
pub fn rules_for_egress_proxy(
    host: &str,
    port: u16,
    sandbox_id: Uuid,
    environment_id: Uuid,
) -> Vec<NetworkRule> {
    let host = host.trim();
    if host.is_empty() || port == 0 {
        return Vec::new();
    }
    vec![NetworkRule {
        id: Uuid::nil(),
        environment_id,
        policy_id: None,
        sandbox_id: Some(sandbox_id),
        name: "Egress proxy".into(),
        action: RuleAction::Allow,
        host: host.to_string(),
        port: Some(i32::from(port)),
        protocol: RuleProtocol::Tcp,
        methods: vec![],
        binaries: vec![],
        matchers: vec![
            Matcher { kind: MatcherKind::Host, value: Some(host.to_string()) },
            Matcher { kind: MatcherKind::Port, value: Some(port.to_string()) },
        ],
        enforcement: Enforcement::Enforced,
        priority: DOCUMENT_ALLOW_PRIORITY - 200,
        enabled: true,
        version: 1,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hot_and_cold_splits_network_from_startup_sections() {
        let spec = serde_json::json!({
            "network_policies": {}, "filesystem_policy": {}, "landlock": {}, "process": {}
        });
        let (hot, cold) = hot_and_cold(&spec);
        assert_eq!(hot, vec!["network_policies"]);
        assert_eq!(cold, vec!["filesystem_policy", "landlock", "process"]);
    }

    use serde_json::json;

    #[test]
    fn the_proxy_hop_is_an_explicit_tcp_allow() {
        let sandbox = Uuid::new_v4();
        let environment = Uuid::new_v4();
        let rules = rules_for_egress_proxy("host.openshell.internal", 18087, sandbox, environment);
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.action, RuleAction::Allow);
        assert_eq!(rule.host, "host.openshell.internal");
        assert_eq!(rule.port, Some(18087));
        // `tcp`, not `rest`: a forward proxy speaks CONNECT, so there is no
        // readable request inside the tunnel for an L7 policy to match.
        assert_eq!(rule.protocol, RuleProtocol::Tcp);
        assert_eq!(rule.sandbox_id, Some(sandbox));
    }

    #[test]
    fn a_proxy_without_a_host_grants_nothing() {
        let rules = rules_for_egress_proxy("  ", 18087, Uuid::new_v4(), Uuid::new_v4());
        assert!(rules.is_empty());
    }

    #[test]
    fn network_is_the_only_hot_reloadable_section() {
        assert!(PolicySection::NetworkPolicies.is_hot_reloadable());
        assert!(!PolicySection::FilesystemPolicy.is_hot_reloadable());
        assert!(!PolicySection::Landlock.is_hot_reloadable());
        assert!(!PolicySection::Process.is_hot_reloadable());
    }

    #[test]
    fn a_network_only_change_on_a_running_sandbox_applies_now() {
        assert_eq!(
            apply_outcome(&[PolicySection::NetworkPolicies], true),
            ApplyOutcome::AppliesNow
        );
    }

    #[test]
    fn a_startup_section_change_on_a_running_sandbox_requires_recreate() {
        assert_eq!(
            apply_outcome(&[PolicySection::FilesystemPolicy], true),
            ApplyOutcome::RequiresRecreate
        );
        assert_eq!(
            apply_outcome(
                &[PolicySection::NetworkPolicies, PolicySection::Process],
                true
            ),
            ApplyOutcome::RequiresRecreate
        );
    }

    #[test]
    fn nothing_running_means_everything_applies() {
        assert_eq!(
            apply_outcome(&[PolicySection::Landlock], false),
            ApplyOutcome::AppliesNow
        );
    }

    #[test]
    fn validation_rejects_a_non_object() {
        assert_eq!(
            validate_document(&json!("nope")),
            Err(PolicyError::NotAnObject)
        );
        assert_eq!(validate_document(&json!({})), Err(PolicyError::Empty));
    }

    #[test]
    fn a_typo_in_a_section_word_is_refused_with_the_reason() {
        let doc = json!({
            "filesystem_policy": {
                "paths": [{ "path": "/workspace", "access": "alow" }]
            }
        });
        let error = validate_document(&doc).expect_err("typo");
        assert_eq!(error.code(), "policy.section_invalid");
        assert!(error.to_string().contains("read"), "{error}");
    }

    #[test]
    fn every_section_vocabulary_is_checked() {
        let landlock = json!({
            "landlock": { "rules": [{ "path": "/home", "access": ["read", "admin"] }] }
        });
        assert!(validate_document(&landlock).is_err());

        let process = json!({ "process": { "max_processes": 0 } });
        assert!(validate_document(&process).is_err());

        let endpoint = json!({
            "network_policies": { "api": { "endpoints": [{ "host": "a b" }] } }
        });
        assert!(validate_document(&endpoint).is_err());

        let action = json!({
            "network_policies": { "api": { "endpoints": [{ "host": "a", "access": "maybe" }] } }
        });
        assert!(validate_document(&action).is_err());
    }

    #[test]
    fn an_empty_section_is_a_valid_statement() {
        let doc = json!({ "filesystem_policy": {} });
        assert_eq!(
            validate_document(&doc),
            Ok(vec![PolicySection::FilesystemPolicy])
        );
    }

    #[test]
    fn the_schema_names_every_field_the_validation_checks() {
        let schema = policy_schema();
        let sections = schema["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), 4);
        for section in sections {
            assert!(section["key"].is_string());
            assert!(section["title"].is_string());
        }
        // The access words in the schema are exactly the words the validation
        // accepts; that is the invariant the form editor stands on.
        let filesystem = sections
            .iter()
            .find(|s| s["key"] == "filesystem_policy")
            .expect("filesystem section");
        let values: Vec<String> = filesystem["fields"][1]["values"]
            .as_array()
            .expect("values")
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
        assert_eq!(
            values,
            PATH_ACCESS.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn validation_reports_the_sections_present() {
        let doc = json!({
            "process": {"run_as_user": "sandbox"},
            "network_policies": {}
        });
        let sections = validate_document(&doc).expect("valid");
        assert_eq!(
            sections,
            vec![PolicySection::Process, PolicySection::NetworkPolicies]
        );
    }

    #[test]
    fn projection_flattens_endpoints_into_rules() {
        let doc = json!({
            "network_policies": {
                "model-api": {
                    "endpoints": [{
                        "host": "api.example.com",
                        "port": 443,
                        "protocol": "rest",
                        "enforcement": "enforced",
                        "access": "allow",
                        "methods": ["GET", "POST"],
                        "binaries": [{"path": "/usr/local/bin/agent"}]
                    }]
                }
            }
        });
        let rules = rules_from_document(&doc);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].host, "api.example.com");
        assert_eq!(rules[0].port, Some(443));
        assert_eq!(rules[0].action, RuleAction::Allow);
        assert_eq!(rules[0].methods, vec!["GET", "POST"]);
        assert_eq!(rules[0].binaries, vec!["/usr/local/bin/agent"]);
    }

    #[test]
    fn projection_skips_an_endpoint_without_a_host() {
        let doc = json!({"network_policies": {"x": {"endpoints": [{"port": 1}]}}});
        assert!(rules_from_document(&doc).is_empty());
    }

    #[test]
    fn audit_enforcement_does_not_block() {
        assert!(!Enforcement::Audit.blocks());
        assert!(Enforcement::Enforced.blocks());
    }

    // ---------------------------------------------------------------
    // Matchers
    // ---------------------------------------------------------------

    fn subject<'a>(
        host: &'a str,
        port: i32,
        protocol: Option<&'a str>,
        method: Option<&'a str>,
    ) -> MatchSubject<'a> {
        MatchSubject {
            host,
            port,
            protocol,
            method,
            binary: None,
        }
    }

    #[test]
    fn a_host_matcher_matches_its_own_name_and_no_other() {
        let matcher = Matcher::new(MatcherKind::Host, Some("api.example.com".into()))
            .expect("valid");
        assert!(matcher.matches(&subject("api.example.com", 443, None, None)));
        assert!(!matcher.matches(&subject("api2.example.com", 443, None, None)));
    }

    #[test]
    fn a_suffix_matcher_covers_subdomains_but_not_lookalikes() {
        let matcher = Matcher::new(MatcherKind::HostSuffix, Some(".example.com".into()))
            .expect("valid");
        assert!(matcher.matches(&subject("api.example.com", 443, None, None)));
        assert!(matcher.matches(&subject("example.com", 443, None, None)));
        assert!(!matcher.matches(&subject("notexample.com", 443, None, None)));
    }

    #[test]
    fn a_cidr_matcher_takes_a_literal_address_and_refuses_a_name() {
        let matcher = Matcher::new(MatcherKind::Cidr, Some("10.0.0.0/8".into()))
            .expect("valid");
        assert!(matcher.matches(&subject("10.42.1.7", 443, None, None)));
        assert!(!matcher.matches(&subject("192.168.0.1", 443, None, None)));
        // A name cannot be inside a prefix; that is the honest answer.
        assert!(!matcher.matches(&subject("example.com", 443, None, None)));
    }

    #[test]
    fn a_port_matcher_takes_one_port_and_a_range() {
        let one = Matcher::new(MatcherKind::Port, Some("443".into())).expect("valid");
        assert!(one.matches(&subject("example.com", 443, None, None)));
        assert!(!one.matches(&subject("example.com", 8443, None, None)));

        let range = Matcher::new(MatcherKind::Port, Some("8000-8100".into()))
            .expect("valid");
        assert!(range.matches(&subject("example.com", 8000, None, None)));
        assert!(range.matches(&subject("example.com", 8100, None, None)));
        assert!(!range.matches(&subject("example.com", 8101, None, None)));
    }

    #[test]
    fn a_protocol_matcher_keeps_the_wildcards_the_vocabulary_had() {
        let http = Matcher::new(MatcherKind::Protocol, Some("http".into())).expect("valid");
        assert!(http.matches(&subject("example.com", 443, Some("https"), None)));
        let tcp = Matcher::new(MatcherKind::Protocol, Some("tcp".into())).expect("valid");
        assert!(tcp.matches(&subject("example.com", 1234, Some("anything"), None)));
    }

    #[test]
    fn a_method_matcher_is_case_insensitive_and_needs_a_method() {
        let matcher = Matcher::new(MatcherKind::Method, Some("POST".into())).expect("valid");
        assert!(matcher.matches(&subject("example.com", 443, None, Some("post"))));
        assert!(!matcher.matches(&subject("example.com", 443, None, Some("GET"))));
        assert!(!matcher.matches(&subject("example.com", 443, None, None)));
    }

    #[test]
    fn any_matches_everything_and_takes_no_value() {
        let matcher = Matcher::new(MatcherKind::Any, None).expect("valid");
        assert!(matcher.matches(&subject("anything.example", 1, None, None)));
        assert!(Matcher::new(MatcherKind::Any, Some("no".into())).is_err());
    }

    #[test]
    fn an_invalid_value_is_refused_at_construction() {
        assert!(Matcher::new(MatcherKind::Cidr, Some("10.0.0.0".into())).is_err());
        assert!(Matcher::new(MatcherKind::Port, Some("0".into())).is_err());
        assert!(Matcher::new(MatcherKind::Port, Some("80-40".into())).is_err());
        assert!(Matcher::new(MatcherKind::Protocol, Some("gopher".into())).is_err());
        assert!(Matcher::new(MatcherKind::Host, Some("with space".into())).is_err());
        assert!(Matcher::new(MatcherKind::Method, Some("GE T".into())).is_err());
    }

    #[test]
    fn a_rule_with_matchers_is_read_by_the_decider() {
        use crate::domain::egress::{decide, CheckRequest, CompiledPolicy};
        let rule = NetworkRule {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: "suffix allow".into(),
            action: RuleAction::Allow,
            host: "*".into(),
            port: None,
            protocol: RuleProtocol::Tcp,
            methods: vec![],
            binaries: vec![],
            matchers: vec![
                Matcher::new(MatcherKind::HostSuffix, Some(".example.com".into()))
                    .expect("valid"),
            ],
            enforcement: Enforcement::Enforced,
            priority: 1,
            enabled: true,
            version: 1,
        };
        let policy = CompiledPolicy {
            environment_id: Uuid::nil(),
            hash: "test".into(),
            compiled_at: Utc::now(),
            rules: vec![rule],
            windows: vec![],
        };
        let request = CheckRequest {
            sandbox_id: Some(Uuid::nil()),
            node_id: None,
            host: "api.example.com".into(),
            port: 443,
            protocol: Some("https".into()),
            method: Some("GET".into()),
            path: None,
            user_agent: None,
            mcp_tool: None,
            mcp_session_id: None,
        };
        let response = decide(&request, &policy, Utc::now());
        assert_eq!(response.decision, crate::domain::egress::Decision::Allow);

        let elsewhere = CheckRequest { host: "other.net".into(), ..request };
        let response = decide(&elsewhere, &policy, Utc::now());
        assert_eq!(response.decision, crate::domain::egress::Decision::Deny);
    }

    #[test]
    fn flat_fields_derive_from_matchers_and_back() {
        let mut rule = NetworkRule {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: "legacy".into(),
            action: RuleAction::Allow,
            host: "api.example.com".into(),
            port: Some(8443),
            protocol: RuleProtocol::Rest,
            methods: vec!["GET".into()],
            binaries: vec!["/usr/local/bin/agent".into()],
            matchers: vec![],
            enforcement: Enforcement::Enforced,
            priority: 1,
            enabled: true,
            version: 1,
        };
        normalise_matchers(&mut rule);
        assert_eq!(rule.matchers.len(), 5);

        let flat = flat_from_matchers(&rule.matchers);
        assert_eq!(flat.host, "api.example.com");
        assert_eq!(flat.port, Some(8443));
        assert_eq!(flat.protocol, RuleProtocol::Rest);
        assert_eq!(flat.methods, vec!["GET".to_string()]);
        assert_eq!(flat.binaries, vec!["/usr/local/bin/agent".to_string()]);
    }

    #[test]
    fn a_policy_document_becomes_rules_behind_the_stored_ones() {
        let policy = Policy {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            name: "defaults".into(),
            scope: PolicyScope::Sandbox,
            spec: json!({ "network_policies": { "models": { "endpoints": [
                { "host": "api.anthropic.com", "port": 443, "access": "allow" },
                { "host": "evil.example", "access": "deny" }
            ], "enforcement": "enforced" } } }),
            version: 3,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let rules = rules_from_policy(&policy);
        assert_eq!(rules.len(), 2);
        let allow = rules.iter().find(|r| r.host == "api.anthropic.com").expect("allow");
        let deny = rules.iter().find(|r| r.host == "evil.example").expect("deny");
        assert!(allow.priority >= DOCUMENT_ALLOW_PRIORITY && deny.priority < allow.priority);
        assert!(allow.priority > 100, "an operator's own rule (default 100) is read first");
        assert_eq!(allow.id, Uuid::nil());
        assert_eq!(allow.policy_id, Some(policy.id));
    }

    #[test]
    fn a_rule_on_a_policy_applies_only_to_sandboxes_that_carry_it() {
        let sandbox = Uuid::new_v4();
        let (mine, other) = (Uuid::new_v4(), Uuid::new_v4());
        let mk = |policy: Option<Uuid>, target: Option<Uuid>| NetworkRule {
            id: Uuid::new_v4(), environment_id: Uuid::nil(), policy_id: policy, sandbox_id: target,
            name: "r".into(), action: RuleAction::Allow, host: "h".into(), port: None,
            protocol: RuleProtocol::Tcp, methods: vec![], binaries: vec![], matchers: vec![],
            enforcement: Enforcement::Enforced, priority: 100, enabled: true, version: 1,
        };
        let kept = rules_for_sandbox_view(
            vec![mk(Some(mine), None), mk(Some(other), None), mk(None, None), mk(None, Some(sandbox)), mk(None, Some(Uuid::new_v4()))],
            sandbox,
            Some(mine),
        );
        assert_eq!(kept.len(), 3, "mine, global, and this sandbox's own");
        assert!(rules_for_sandbox_view(vec![mk(Some(mine), None)], sandbox, None).is_empty());
    }

    #[test]
    fn a_dns_suffix_covers_the_domain_and_its_subdomains_but_not_a_lookalike() {
        assert!(dns_pattern_matches(".tracker.example", "tracker.example"));
        assert!(dns_pattern_matches(".tracker.example", "a.b.tracker.example"));
        assert!(!dns_pattern_matches(".tracker.example", "nottracker.example"));
        assert!(dns_pattern_matches("exact.example", "EXACT.example."));
        assert!(!dns_pattern_matches("exact.example", "sub.exact.example"));
    }

    #[test]
    fn the_lowest_priority_dns_rule_that_applies_decides() {
        let sandbox = Uuid::new_v4();
        let rule = |action: RuleAction, pattern: &str, priority: i32, sandbox_id: Option<Uuid>| DnsRule {
            id: Uuid::new_v4(), environment_id: Uuid::nil(), policy_id: None, sandbox_id, action,
            pattern: pattern.into(), priority, enabled: true, version: 1,
        };
        let rules = vec![
            rule(RuleAction::Allow, ".example.net", 200, None),
            rule(RuleAction::Deny, ".example.net", 100, None),
            rule(RuleAction::Deny, ".other.net", 1, Some(Uuid::new_v4())),
        ];
        assert_eq!(dns_rule_for(&rules, sandbox, None, "a.example.net").map(|r| r.action), Some(RuleAction::Deny));
        assert!(dns_rule_for(&rules, sandbox, None, "a.other.net").is_none(), "another sandbox's rule does not apply");
    }

    #[test]
    fn an_endpoint_url_gives_its_host_and_port() {
        assert_eq!(endpoint_host_port("http://10.90.92.30:20128/v1"), Some(("10.90.92.30".into(), 20128)));
        assert_eq!(endpoint_host_port("https://api.example.com/v1"), Some(("api.example.com".into(), 443)));
        assert_eq!(endpoint_host_port("http://gw.local/v1"), Some(("gw.local".into(), 80)));
        assert_eq!(endpoint_host_port("nonsense"), None);
    }

    #[test]
    fn a_document_is_classified_by_the_sections_it_carries() {
        // The badge is derived, not stored: adding a filesystem section makes a
        // policy an OpenShell policy without anyone updating a column.
        let network_only = serde_json::json!({
            "network_policies": { "api": { "endpoints": [] } }
        });
        assert_eq!(classify(&network_only), PolicyKind::Network);
        assert_eq!(document_sections(&network_only), vec!["network_policies"]);

        let whole_fence = serde_json::json!({
            "filesystem_policy": { "paths": [] },
            "network_policies": {}
        });
        assert_eq!(classify(&whole_fence), PolicyKind::Openshell);
        assert_eq!(
            document_sections(&whole_fence),
            vec!["filesystem_policy", "network_policies"]
        );

        // A document that is not an object is not classified as anything it is
        // not: the network default is the narrower claim.
        assert_eq!(classify(&serde_json::json!("nonsense")), PolicyKind::Network);
    }
    #[test]
    fn the_accounting_proxy_is_allowed_for_that_sandbox_only() {
        let sandbox = Uuid::new_v4();
        let rules = rules_for_accounting_proxy("http://host.openshell.internal:18099/", sandbox, Uuid::nil());
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].host, "host.openshell.internal");
        assert_eq!(rules[0].port, Some(18099));
        assert_eq!(rules[0].sandbox_id, Some(sandbox));
        assert!(rules_for_accounting_proxy("not a url", sandbox, Uuid::nil()).is_empty());
    }

}
