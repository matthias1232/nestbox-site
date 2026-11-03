//! Nestbox network rules, compiled into an OpenShell sandbox policy.
//!
//! OpenShell denies every connection that no `network_policies` entry allows, so
//! the compiler only has to write the *allows*. A Nestbox deny rule is honoured
//! twice over: it never becomes an entry, and it removes an allow for the same
//! destination that sorts behind it. What a rule cannot express in OpenShell is
//! reported in [`Compiled::skipped`] instead of being dropped silently.
//!
//! The result is a plain `serde_json::Value` (JSON is YAML), so the runner can write
//! it straight to a file for `openshell policy set`.

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::policy::{Enforcement, MatcherKind, NetworkRule, RuleAction, RuleProtocol};

#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    pub document: Value,
    /// Rules that could not be expressed, with the reason.
    pub skipped: Vec<(String, String)>,
}

impl Compiled {
    /// A stable fingerprint, so the sync only pushes when something changed.
    pub fn fingerprint(&self) -> String {
        let text = serde_json::to_string(&self.document).unwrap_or_default();
        let digest = Sha256::digest(text.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Compile the effective rules of one sandbox.
pub fn compile(rules: &[NetworkRule]) -> Compiled {
    let mut skipped = Vec::new();
    let mut entries = Map::new();
    let denies: Vec<&NetworkRule> = rules
        .iter()
        .filter(|rule| rule.enabled && rule.action == RuleAction::Deny)
        .collect();

    for (index, rule) in rules.iter().enumerate() {
        if !rule.enabled || rule.action != RuleAction::Allow {
            continue;
        }
        if is_denied(rule, &denies) {
            skipped.push((rule.name.clone(), "a deny rule for the same destination comes first".into()));
            continue;
        }
        match endpoint_for(rule) {
            Ok(endpoint) => {
                let mut key = key_for(&rule.name, index);
                let mut suffix = b'b';
                while entries.contains_key(&key) {
                    key = format!("{}_{}", key_for(&rule.name, index), suffix as char);
                    suffix = suffix.saturating_add(1).min(b'z');
                    if suffix == b'z' && entries.contains_key(&key) {
                        key.push('z');
                    }
                }
                entries.insert(key, json!({ "name": rule.name, "endpoints": [endpoint], "binaries": binaries_for(rule) }));
            }
            Err(reason) => skipped.push((rule.name.clone(), reason)),
        }
    }

    // Network rules only. Filesystem, Landlock and process settings take effect at
    // startup and cannot be changed on a live sandbox, so the runner keeps the
    // sandbox's own and replaces just this section.
    let document = json!({ "version": 1, "network_policies": Value::Object(entries) });
    Compiled { document, skipped }
}

/// The policy of a sandbox whose rules are suspended: everything allowed, everything logged.
///
/// Used for the `off_allow_all` egress mode and for an open window on the OpenShell
/// channel (ADR 0054). The allow-all entry is an address range rather than a host
/// wildcard, because a wildcard host is not a destination OpenShell can resolve;
/// every base entry stays, so the policy only grows. `enforcement: audit` makes the
/// supervisor report instead of refuse, which is the whole point of the mode.
pub fn allow_all(base: &Compiled) -> Compiled {
    let mut document = base.document.clone();
    let Some(policies) = document.get_mut("network_policies").and_then(Value::as_object_mut) else {
        return base.clone();
    };
    // Idempotent: a second pass (every 30 s) must not stack entries.
    if policies.contains_key("nb_allow_all") {
        return Compiled { document, skipped: base.skipped.clone() };
    }
    policies.insert(
        "nb_allow_all".to_string(),
        json!({
            "name": "Everything allowed (evaluation mode)",
            "endpoints": [{
                "allowed_ips": ["0.0.0.0/0"],
                "protocol": "rest",
                "enforcement": "audit",
                "access": "full",
            }],
            "binaries": [{ "path": "/**" }],
        }),
    );
    Compiled { document, skipped: base.skipped.clone() }
}

fn is_denied(allow: &NetworkRule, denies: &[&NetworkRule]) -> bool {
    denies.iter().any(|deny| {
        deny.priority <= allow.priority
            && !deny.host.is_empty()
            && deny.host != "*"
            && deny.host == allow.host
            && (deny.port.is_none() || deny.port == allow.port)
    })
}

fn key_for(name: &str, index: usize) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let slug = slug.trim_matches('_');
    let slug: String = slug.chars().take(60).collect();
    // No running number: a long name of letters and digits looks like a credential to the
    // log redaction, which would blank the policy name in every connection event. A clash
    // gets a letter suffix in `compile`.
    let _ = index;
    format!("nb_{slug}")
}

fn binaries_for(rule: &NetworkRule) -> Vec<Value> {
    if rule.binaries.is_empty() {
        // "Any program": OpenShell matches nothing for an empty list.
        return vec![json!({ "path": "/**" })];
    }
    rule.binaries.iter().map(|path| json!({ "path": path })).collect()
}

fn endpoint_for(rule: &NetworkRule) -> Result<Value, String> {
    let mut host = rule.host.trim().to_string();
    let mut cidr: Option<String> = None;
    let mut port = rule.port;
    let mut port_range: Option<(u16, u16)> = None;
    for matcher in &rule.matchers {
        let value = matcher.value.clone().unwrap_or_default();
        match matcher.kind {
            MatcherKind::Host if host.is_empty() || host == "*" => host = value,
            MatcherKind::HostSuffix => host = format!("*.{}", value.trim_start_matches('.')),
            MatcherKind::Cidr => cidr = Some(value),
            MatcherKind::Port if port.is_none() => {
                if let Some((low, high)) = value.split_once('-') {
                    if let (Ok(low), Ok(high)) = (low.parse::<u16>(), high.parse::<u16>()) {
                        port_range = Some((low, high));
                    }
                } else {
                    port = value.parse().ok();
                }
            }
            _ => {}
        }
    }
    let mut endpoint = Map::new();
    if host.is_empty() || host == "*" {
        let Some(cidr) = cidr else {
            return Err("no host or address range to allow".into());
        };
        endpoint.insert("allowed_ips".into(), json!([cidr]));
    } else {
        if host.starts_with("*.") && host.matches('.').count() < 2 {
            return Err(format!("wildcard '{host}' has fewer than three labels"));
        }
        endpoint.insert("host".into(), json!(host));
    }
    match (port, port_range) {
        (Some(port), _) => {
            endpoint.insert("port".into(), json!(port));
        }
        (None, Some((low, high))) if high >= low && high - low < 64 => {
            endpoint.insert("ports".into(), json!((low..=high).collect::<Vec<_>>()));
        }
        (None, Some(_)) => return Err("port range wider than 64 ports".into()),
        (None, None) => {
            endpoint.insert("port".into(), json!(443));
        }
    }
    let enforce = if rule.enforcement == Enforcement::Enforced { "enforce" } else { "audit" };
    match rule.protocol {
        RuleProtocol::Rest | RuleProtocol::Http => {
            endpoint.insert("protocol".into(), json!("rest"));
            endpoint.insert("enforcement".into(), json!(enforce));
            if rule.methods.is_empty() {
                endpoint.insert("access".into(), json!("full"));
            } else {
                let allowed: Vec<Value> = rule
                    .methods
                    .iter()
                    .map(|method| json!({ "allow": { "method": method.to_uppercase(), "path": "/**" } }))
                    .collect();
                endpoint.insert("rules".into(), Value::Array(allowed));
            }
        }
        RuleProtocol::Websocket => {
            endpoint.insert("protocol".into(), json!("websocket"));
            endpoint.insert("enforcement".into(), json!(enforce));
            endpoint.insert("access".into(), json!("full"));
        }
        RuleProtocol::Graphql => {
            endpoint.insert("protocol".into(), json!("graphql"));
            endpoint.insert("enforcement".into(), json!(enforce));
            endpoint.insert("access".into(), json!("full"));
        }
        RuleProtocol::Mcp => {
            endpoint.insert("protocol".into(), json!("mcp"));
            endpoint.insert("enforcement".into(), json!(enforce));
            // OpenShell allows only the newest MCP revision by default; agents in the
            // field still speak the earlier ones (the revisions OpenShell supports).
            endpoint.insert(
                "mcp".into(),
                json!({
                    "allow_all_known_mcp_methods": true,
                    "versions": ["2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"],
                }),
            );
        }
        RuleProtocol::Tcp => {
            if endpoint.contains_key("allowed_ips") {
                return Err("a native TCP rule needs a hostname".into());
            }
            if host.parse::<std::net::IpAddr>().is_ok() {
                // OpenShell refuses `protocol: tcp` for an IP literal (it would bypass
                // policy DNS). The destination is still allowed, relayed as is.
                endpoint.insert("tls".into(), json!("skip"));
            } else {
                endpoint.insert("protocol".into(), json!("tcp"));
            }
        }
        // JSON-RPC needs explicit method rules, which a flat rule does not carry:
        // fall back to a plain endpoint rather than refuse the destination.
        RuleProtocol::Jsonrpc => {}
    }
    Ok(Value::Object(endpoint))
}

/// The policy of a sandbox whose rules are suspended: the named destinations are
/// allowed in audit mode, so the supervisor records them instead of blocking.
///
/// OpenShell 0.1.6 has no host wildcard, so there is no single entry that allows
/// every destination. `host: "*"` and `"**"` are rejected ("middle DNS label
/// wildcards must be the entire label '*' and recursive '**' is only allowed as
/// the entire first label"), `"*.*"` is rejected as a TLD wildcard, and a
/// hostless `allowed_ips` endpoint only matches through OpenShell's own forward
/// proxy, which no client in these images uses. Allowing the destinations that
/// were actually seen therefore stays the honest option: a host the sandbox has
/// not talked to yet stays blocked until its first attempt appears in telemetry.
///
/// Every base entry is kept — a suspended rule set may suspend a deny, not the
/// permissions the sandbox needs to run at all. The call is idempotent: a
/// destination that an audit entry already allows is not added again.
pub fn allow_hosts(base: &Compiled, hosts: &[(String, i32)]) -> Compiled {
    let mut document = base.document.clone();
    let Some(policies) = document.get_mut("network_policies").and_then(Value::as_object_mut) else {
        return base.clone();
    };
    let mut taken = audit_destinations(policies);
    let mut added: Vec<(String, i32)> = Vec::new();
    for (host, port) in hosts {
        let host = host.trim();
        // A name with a wildcard is already an entry of its own, an empty one is
        // not a destination, and OpenShell refuses a port outside the range — one
        // bad row would fail the whole policy push.
        if host.is_empty()
            || host.contains('*')
            || *port < 1
            || *port > 65535
            || !taken.insert((host.to_string(), *port))
        {
            continue;
        }
        added.push((host.to_string(), *port));
    }
    for (index, (host, port)) in added.iter().enumerate() {
        let key = allow_key(index, policies);
        policies.insert(
            key,
            json!({
                "name": format!("Nestbox allow all: {host}:{port}"),
                "endpoints": [allow_endpoint(host, *port)],
                "binaries": [{ "path": "/**" }],
            }),
        );
    }
    Compiled { document, skipped: base.skipped.clone() }
}

/// Section and entry keys in the order the runner writes them; other keys follow alphabetically.
const YAML_KEY_ORDER: [&str; 14] = [
    "version",
    "filesystem_policy",
    "landlock",
    "process",
    "network_policies",
    "name",
    "endpoints",
    "binaries",
    "host",
    "port",
    "protocol",
    "access",
    "enforcement",
    "path",
];

/// Render a compiled document as YAML text, deterministically.
///
/// The control plane has no YAML dependency, and the document is plain JSON, so a small
/// emitter is enough: fixed key order, two-space indent, strings quoted when YAML would
/// misread them. The preview shows this text, so equal documents must give equal text.
pub fn to_yaml(compiled: &Compiled) -> String {
    let mut out = String::new();
    match &compiled.document {
        Value::Object(map) if !map.is_empty() => yaml_map(map, 0, &mut out),
        other => {
            out.push_str(&yaml_scalar(other));
            out.push('\n');
        }
    }
    out
}

fn yaml_sorted_keys(map: &Map<String, Value>) -> Vec<&String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|key| {
        let rank = YAML_KEY_ORDER
            .iter()
            .position(|known| *known == key.as_str())
            .unwrap_or(YAML_KEY_ORDER.len());
        (rank, key.to_string())
    });
    keys
}

fn yaml_is_block(value: &Value) -> bool {
    match value {
        Value::Object(map) => !map.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    }
}

fn yaml_map(map: &Map<String, Value>, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    for key in yaml_sorted_keys(map) {
        let value = &map[key];
        out.push_str(&pad);
        out.push_str(&yaml_string(key));
        out.push(':');
        if yaml_is_block(value) {
            out.push('\n');
            yaml_block(value, indent + 2, out);
        } else {
            out.push(' ');
            out.push_str(&yaml_scalar(value));
            out.push('\n');
        }
    }
}

fn yaml_block(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Object(map) => yaml_map(map, indent, out),
        Value::Array(items) => {
            let pad = " ".repeat(indent);
            for item in items {
                if yaml_is_block(item) {
                    // "- " then the nested block with its first line pulled up onto the dash.
                    let mut nested = String::new();
                    yaml_block(item, indent + 2, &mut nested);
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(nested.get(indent + 2..).unwrap_or(""));
                } else {
                    out.push_str(&pad);
                    out.push_str("- ");
                    out.push_str(&yaml_scalar(item));
                    out.push('\n');
                }
            }
        }
        other => {
            out.push_str(&" ".repeat(indent));
            out.push_str(&yaml_scalar(other));
            out.push('\n');
        }
    }
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => yaml_string(text),
        Value::Array(_) => "[]".to_string(),
        Value::Object(_) => "{}".to_string(),
    }
}

/// A string, quoted when plain YAML would change its meaning.
fn yaml_string(text: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    let reserved = matches!(lowered.as_str(), "" | "null" | "true" | "false" | "yes" | "no" | "on" | "off" | "~")
        || text.parse::<f64>().is_ok();
    let risky = text.contains(": ")
        || text.contains(':') && text.ends_with(':')
        || text.contains('#')
        || text.contains('\n')
        || text.contains('"')
        || text.starts_with(|c: char| c.is_whitespace() || "-?:,[]{}&*!|>'%@`".contains(c))
        || text.ends_with(char::is_whitespace);
    if reserved || risky || text.contains(':') {
        serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
    } else {
        text.to_string()
    }
}

/// The destinations that an audit entry of this policy already allows.
///
/// An entry compiled from a rule can be in audit mode too; adding a second entry
/// for the same destination would only make the policy longer.
fn audit_destinations(policies: &Map<String, Value>) -> std::collections::HashSet<(String, i32)> {
    let mut out = std::collections::HashSet::new();
    for entry in policies.values() {
        let Some(endpoints) = entry.get("endpoints").and_then(Value::as_array) else {
            continue;
        };
        for endpoint in endpoints {
            if endpoint.get("enforcement").and_then(Value::as_str) != Some("audit") {
                continue;
            }
            let Some(host) = endpoint.get("host").and_then(Value::as_str) else {
                continue;
            };
            if let Some(port) = endpoint.get("port").and_then(Value::as_i64) {
                out.insert((host.to_string(), port as i32));
            }
        }
    }
    out
}

/// A key for one added entry, free of digits.
///
/// The log redaction blanks names that look like credentials, and a name with
/// digits in it does — the same reason `key_for` avoids them.
fn allow_key(index: usize, policies: &Map<String, Value>) -> String {
    let mut suffix = String::new();
    let mut value = index;
    loop {
        suffix.insert(0, (b'a' + (value % 26) as u8) as char);
        if value < 26 {
            break;
        }
        value = value / 26 - 1;
    }
    let key = format!("nestbox_allow_{suffix}");
    if policies.contains_key(&key) {
        return format!("{key}_x");
    }
    key
}

/// One audit endpoint for a named destination.
fn allow_endpoint(host: &str, port: i32) -> Value {
    let mut endpoint = Map::new();
    endpoint.insert("host".into(), json!(host));
    endpoint.insert("port".into(), json!(port));
    if host.parse::<std::net::IpAddr>().is_ok() {
        // OpenShell refuses `protocol: rest` for an address literal (it bypasses
        // policy DNS); the connection is relayed unread, which audit mode accepts.
        endpoint.insert("tls".into(), json!("skip"));
    } else {
        endpoint.insert("protocol".into(), json!("rest"));
        endpoint.insert("enforcement".into(), json!("audit"));
        endpoint.insert("access".into(), json!("full"));
    }
    Value::Object(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::Matcher;
    use uuid::Uuid;

    fn rule(name: &str, action: RuleAction, host: &str, port: Option<i32>, protocol: RuleProtocol) -> NetworkRule {
        NetworkRule {
            id: Uuid::nil(),
            environment_id: Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: name.into(),
            action,
            host: host.into(),
            port,
            protocol,
            methods: vec![],
            binaries: vec![],
            matchers: vec![],
            enforcement: Enforcement::Enforced,
            priority: 100,
            enabled: true,
            version: 1,
        }
    }

    fn entries(compiled: &Compiled) -> &Map<String, Value> {
        compiled.document["network_policies"].as_object().unwrap()
    }

    #[test]
    fn an_empty_rule_set_is_a_policy_that_allows_nothing() {
        let compiled = compile(&[]);
        assert!(entries(&compiled).is_empty());
        assert_eq!(compiled.document["version"], 1);
    }

    #[test]
    fn an_allow_becomes_an_endpoint_for_any_program() {
        let compiled = compile(&[rule("Model", RuleAction::Allow, "10.90.92.30", Some(20128), RuleProtocol::Http)]);
        let entry = entries(&compiled).values().next().unwrap();
        assert_eq!(entry["endpoints"][0]["host"], "10.90.92.30");
        assert_eq!(entry["endpoints"][0]["port"], 20128);
        assert_eq!(entry["endpoints"][0]["protocol"], "rest");
        assert_eq!(entry["endpoints"][0]["access"], "full");
        assert_eq!(entry["binaries"][0]["path"], "/**");
    }

    #[test]
    fn a_tcp_rule_for_an_ip_literal_is_relayed_without_a_protocol() {
        let compiled = compile(&[rule("Model", RuleAction::Allow, "10.90.92.30", Some(20128), RuleProtocol::Tcp)]);
        let endpoint = &entries(&compiled).values().next().unwrap()["endpoints"][0];
        assert!(endpoint.get("protocol").is_none());
        assert_eq!(endpoint["tls"], "skip");
        let named = compile(&[rule("Db", RuleAction::Allow, "db.example.com", Some(5432), RuleProtocol::Tcp)]);
        assert_eq!(entries(&named).values().next().unwrap()["endpoints"][0]["protocol"], "tcp");
    }

    #[test]
    fn an_mcp_rule_allows_the_revisions_agents_speak() {
        let compiled = compile(&[rule("Hub", RuleAction::Allow, "host.openshell.internal", Some(18088), RuleProtocol::Mcp)]);
        let mcp = &entries(&compiled).values().next().unwrap()["endpoints"][0]["mcp"];
        assert_eq!(mcp["allow_all_known_mcp_methods"], true);
        assert!(mcp["versions"].as_array().unwrap().iter().any(|v| v == "2025-06-18"));
    }

    #[test]
    fn methods_become_request_rules_and_binaries_are_kept() {
        let mut r = rule("Read", RuleAction::Allow, "api.example.com", Some(443), RuleProtocol::Rest);
        r.methods = vec!["get".into(), "head".into()];
        r.binaries = vec!["/usr/bin/curl".into()];
        let compiled = compile(&[r]);
        let entry = entries(&compiled).values().next().unwrap();
        assert_eq!(entry["endpoints"][0]["rules"][0]["allow"]["method"], "GET");
        assert_eq!(entry["endpoints"][0]["rules"][1]["allow"]["method"], "HEAD");
        assert_eq!(entry["binaries"][0]["path"], "/usr/bin/curl");
    }

    #[test]
    fn a_deny_is_never_an_entry_and_removes_a_matching_allow() {
        let mut deny = rule("Block", RuleAction::Deny, "bad.example.com", None, RuleProtocol::Tcp);
        deny.priority = 50;
        let allow = rule("Allow", RuleAction::Allow, "bad.example.com", Some(443), RuleProtocol::Rest);
        let other = rule("Other", RuleAction::Allow, "ok.example.com", Some(443), RuleProtocol::Rest);
        let compiled = compile(&[deny, allow, other]);
        assert_eq!(entries(&compiled).len(), 1);
        assert_eq!(compiled.skipped.len(), 1);
    }

    #[test]
    fn a_host_suffix_becomes_a_wildcard_and_a_short_one_is_reported() {
        let mut ok = rule("Suffix", RuleAction::Allow, "", Some(443), RuleProtocol::Rest);
        ok.matchers = vec![Matcher { kind: MatcherKind::HostSuffix, value: Some(".example.com".into()) }];
        let mut short = rule("Short", RuleAction::Allow, "", Some(443), RuleProtocol::Rest);
        short.matchers = vec![Matcher { kind: MatcherKind::HostSuffix, value: Some("com".into()) }];
        let compiled = compile(&[ok, short]);
        assert_eq!(entries(&compiled).values().next().unwrap()["endpoints"][0]["host"], "*.example.com");
        assert_eq!(compiled.skipped.len(), 1);
    }

    #[test]
    fn policy_names_carry_no_digits_the_redaction_would_mistake_for_a_credential() {
        let compiled = compile(&[
            rule("Safe defaults (best practice): package-registries", RuleAction::Allow, "a.example.com", Some(443), RuleProtocol::Rest),
            rule("Safe defaults (best practice): package-registries", RuleAction::Allow, "b.example.com", Some(443), RuleProtocol::Rest),
        ]);
        let keys: Vec<&String> = entries(&compiled).keys().collect();
        assert_eq!(keys.len(), 2, "two rules of one name both survive");
        assert!(keys.iter().all(|key| key.starts_with("nb_") && !key.chars().any(|c| c.is_ascii_digit())), "{keys:?}");
        let masked = crate::security::redact(keys[0]);
        assert_eq!(&masked, keys[0], "the redaction leaves a policy name alone");
    }

    #[test]
    fn a_disabled_rule_and_a_rule_without_destination_are_not_written() {
        let mut off = rule("Off", RuleAction::Allow, "a.example.com", Some(443), RuleProtocol::Rest);
        off.enabled = false;
        let nothing = rule("Nothing", RuleAction::Allow, "", None, RuleProtocol::Rest);
        let compiled = compile(&[off, nothing]);
        assert!(entries(&compiled).is_empty());
        assert_eq!(compiled.skipped.len(), 1);
    }

    #[test]
    fn a_suspended_sandbox_keeps_its_rules_and_logs_the_seen_hosts() {
        let compiled = compile(&[rule("Model", RuleAction::Allow, "api.example.com", Some(443), RuleProtocol::Rest)]);
        let allowed = allow_hosts(&compiled, &[("example.org".into(), 80)]);
        assert_eq!(entries(&allowed).len(), 2, "the rule entry stays next to the added one");
        assert_eq!(allowed.skipped, compiled.skipped);
        let added = &entries(&allowed)["nestbox_allow_a"];
        assert_eq!(added["endpoints"][0]["host"], "example.org");
        assert_eq!(added["endpoints"][0]["port"], 80);
        assert_eq!(added["endpoints"][0]["enforcement"], "audit");
        assert_eq!(added["endpoints"][0]["access"], "full");
        assert_eq!(added["binaries"][0]["path"], "/**");
        assert!(entries(&allowed).keys().all(|key| !key.chars().any(|c| c.is_ascii_digit())));
    }

    #[test]
    fn allowing_the_seen_hosts_twice_adds_no_second_entry() {
        let compiled = compile(&[]);
        let hosts = vec![("example.org".to_string(), 443), ("example.org".to_string(), 443)];
        let once = allow_hosts(&compiled, &hosts);
        let twice = allow_hosts(&once, &hosts);
        assert_eq!(entries(&once).len(), 1, "one destination is one entry, not one per call");
        assert_eq!(entries(&twice).len(), 1);
        assert_eq!(once.fingerprint(), twice.fingerprint());
        let more = allow_hosts(&once, &[("example.net".to_string(), 80)]);
        assert_eq!(entries(&more).len(), 2, "a new destination is added next to the old one");
    }

    #[test]
    fn a_destination_an_audit_rule_already_allows_is_not_added_again() {
        let mut audit = rule("Audit", RuleAction::Allow, "example.org", Some(443), RuleProtocol::Rest);
        audit.enforcement = Enforcement::Audit;
        let compiled = compile(&[audit]);
        let allowed = allow_hosts(&compiled, &[("example.org".to_string(), 443)]);
        assert_eq!(entries(&allowed).len(), 1);
        assert_eq!(entries(&allowed).values().next().unwrap()["name"], "Audit");
    }

    #[test]
    fn a_wildcard_or_empty_destination_is_not_a_new_entry() {
        let allowed = allow_hosts(
            &compile(&[]),
            &[
                ("*".into(), 443),
                ("".into(), 443),
                ("*.example.com".into(), 443),
                ("example.org".into(), 0),
                ("example.org".into(), 70000),
            ],
        );
        assert!(entries(&allowed).is_empty());
    }

    #[test]
    fn an_address_literal_is_relayed_instead_of_inspected() {
        let allowed = allow_hosts(&compile(&[]), &[("10.0.0.5".to_string(), 8080)]);
        let endpoint = &entries(&allowed).values().next().unwrap()["endpoints"][0];
        assert!(endpoint.get("protocol").is_none());
        assert_eq!(endpoint["tls"], "skip");
        assert_eq!(endpoint["port"], 8080);
    }

    #[test]
    fn the_fingerprint_changes_with_the_rules_and_not_otherwise() {
        let a = compile(&[rule("A", RuleAction::Allow, "a.example.com", Some(443), RuleProtocol::Rest)]);
        let a_again = compile(&[rule("A", RuleAction::Allow, "a.example.com", Some(443), RuleProtocol::Rest)]);
        let b = compile(&[rule("A", RuleAction::Allow, "b.example.com", Some(443), RuleProtocol::Rest)]);
        assert_eq!(a.fingerprint(), a_again.fingerprint());
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}

#[cfg(test)]
mod allow_all_tests {
    use super::*;
    use crate::domain::policy::{Enforcement, RuleProtocol};

    fn allow_rule(name: &str, host: &str) -> NetworkRule {
        NetworkRule {
            id: uuid::Uuid::new_v4(),
            environment_id: uuid::Uuid::nil(),
            policy_id: None,
            sandbox_id: None,
            name: name.to_string(),
            action: RuleAction::Allow,
            host: host.to_string(),
            port: Some(443),
            protocol: RuleProtocol::Rest,
            methods: vec![],
            binaries: vec![],
            matchers: vec![],
            enforcement: Enforcement::Enforced,
            priority: 10,
            enabled: true,
            version: 1,
        }
    }

    #[test]
    fn allow_all_keeps_the_rules_and_adds_one_audited_entry() {
        let base = compile(&[allow_rule("omniroute", "10.90.92.30")]);
        let wide = allow_all(&base);
        let policies = wide.document["network_policies"].as_object().expect("policies");
        assert!(policies.contains_key("nb_allow_all"));
        assert_eq!(policies.len(), base.document["network_policies"].as_object().expect("base").len() + 1);
        let entry = &policies["nb_allow_all"]["endpoints"][0];
        assert_eq!(entry["enforcement"], "audit");
        assert_eq!(entry["allowed_ips"][0], "0.0.0.0/0");
    }

    #[test]
    fn allow_all_is_idempotent() {
        let base = compile(&[]);
        let once = allow_all(&base);
        let twice = allow_all(&once);
        assert_eq!(once.document, twice.document);
    }

    #[test]
    fn the_fingerprint_changes_so_the_sync_pushes() {
        let base = compile(&[allow_rule("omniroute", "10.90.92.30")]);
        assert_ne!(base.fingerprint(), allow_all(&base).fingerprint());
    }

    #[test]
    fn yaml_writes_sections_in_a_fixed_order_with_nested_lists() {
        let compiled = Compiled {
            document: json!({
                "network_policies": { "github": { "name": "github", "endpoints": [{ "port": 443, "host": "github.com" }], "binaries": [{ "path": "/usr/bin/git" }] } },
                "version": 1
            }),
            skipped: Vec::new(),
        };
        let expected = "version: 1\nnetwork_policies:\n  github:\n    name: github\n    endpoints:\n      - host: github.com\n        port: 443\n    binaries:\n      - path: /usr/bin/git\n";
        assert_eq!(to_yaml(&compiled), expected);
    }

    #[test]
    fn yaml_quotes_strings_it_would_misread() {
        assert_eq!(yaml_string("a: b"), "\"a: b\"");
        assert_eq!(yaml_string("#tag"), "\"#tag\"");
        assert_eq!(yaml_string(" lead"), "\" lead\"");
        assert_eq!(yaml_string("443"), "\"443\"");
        assert_eq!(yaml_string("true"), "\"true\"");
        assert_eq!(yaml_string("plain.example.com"), "plain.example.com");
    }

    #[test]
    fn yaml_of_an_empty_section_is_flow_style_and_is_deterministic() {
        let compiled = compile(&[]);
        assert_eq!(to_yaml(&compiled), "version: 1\nnetwork_policies: {}\n");
        assert_eq!(to_yaml(&compiled), to_yaml(&compiled));
    }
}
