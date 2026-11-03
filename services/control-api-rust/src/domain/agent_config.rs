//! The configuration files an agent starts with, so it works the first time.
//!
//! An agent in a sandbox needs to know three things it cannot discover: the model
//! endpoint to use, that its traffic goes through the egress proxy, and which
//! model to ask for. The environment carries the endpoint and the credential; the
//! files here carry the same facts in the shape each agent reads at startup
//! (`~/.claude/settings.json`, `~/.config/opencode/opencode.json`), so a user who
//! opens a shell and types `claude` or `opencode` is already connected, and an
//! agent that ignores the environment is too.
//!
//! Pure: the inputs are the environment the sandbox was given, the outputs are
//! `(path under $HOME, content)` pairs. No secret value is written to a file: the
//! credential stays in the environment and the files refer to it.

use std::collections::BTreeMap;

use serde_json::json;

/// One file to place under the sandbox user's home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    /// Relative to `$HOME`, e.g. `.claude/settings.json`.
    pub path: String,
    pub content: String,
    /// The file is state the agent also writes (its login, its history): an existing
    /// copy is merged into rather than replaced, so a re-applied configuration cannot
    /// sign the user out.
    pub merge_json: bool,
}

fn get<'a>(environment: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    environment.get(key).map(String::as_str).filter(|value| !value.is_empty())
}

/// The files for a pack, from the environment its sandbox is created with.
///
/// A pack with nothing to configure (or a sandbox with no model endpoint) gets no
/// files rather than empty ones: an empty config would shadow the agent's own
/// defaults.
pub fn files_for(pack_key: &str, environment: &BTreeMap<String, String>) -> Vec<AgentFile> {
    files_for_with_servers(pack_key, environment, &[])
}

/// One MCP server as an agent's startup file needs it.
///
/// A locator-free shape on purpose: the token is named (an environment
/// variable), never carried. A config file that held the value would put a
/// credential on a persistent home, where it would outlive the sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerConfig {
    pub key: String,
    /// `stdio`, `http` or `sse`.
    pub transport: String,
    pub endpoint: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    /// The environment variable an HTTP server authenticates with.
    pub token_env: Option<String>,
    /// The sandbox name, sent as a header so a shared server can resolve which
    /// sandbox is asking without trusting the request body.
    pub sandbox: Option<String>,
}

/// The files for a pack, including the MCP servers attached to this sandbox.
///
/// A server is only written when it is attached: attaching it is the act that
/// grants the tool, so a server that was detached must disappear from the
/// config on the next re-apply rather than linger.
pub fn files_for_with_servers(
    pack_key: &str,
    environment: &BTreeMap<String, String>,
    servers: &[McpServerConfig],
) -> Vec<AgentFile> {
    match pack_key {
        "claude-code" => claude_code(environment, servers),
        "opencode" | "openchamber" => opencode(environment, servers),
        _ => Vec::new(),
    }
}

fn claude_code(environment: &BTreeMap<String, String>, servers: &[McpServerConfig]) -> Vec<AgentFile> {
    let mut env = serde_json::Map::new();
    // Through the egress proxy, like every other request from this sandbox.
    for key in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY"] {
        if let Some(value) = get(environment, key) {
            env.insert(key.to_string(), json!(value));
        }
    }
    if let Some(url) = get(environment, "ANTHROPIC_BASE_URL") {
        env.insert("ANTHROPIC_BASE_URL".into(), json!(url));
    }
    // A granted model wins over the profile's single default: once the sandbox may
    // use a set of models, the one the console marked as default is what a new
    // session should ask for, and the environment cannot know that on its own.
    let model = match (get(environment, "NESTBOX_MODEL_DEFAULT"), get(environment, "ANTHROPIC_BASE_URL")) {
        (Some(default), Some(_)) => Some(default),
        _ => get(environment, "ANTHROPIC_MODEL"),
    };
    if let Some(model) = model {
        env.insert("ANTHROPIC_MODEL".into(), json!(model));
        env.insert("ANTHROPIC_SMALL_FAST_MODEL".into(), json!(model));
    }
    // The control plane is the only party that should see what this agent does.
    env.insert("DISABLE_TELEMETRY".into(), json!("1"));
    env.insert("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(), json!("1"));
    env.insert("DISABLE_AUTOUPDATER".into(), json!("1"));

    let settings = json!({
        "env": env,
        // The sandbox is the boundary (filesystem policy, egress policy), so the
        // agent is not asked to confirm every command inside it.
        "permissions": { "defaultMode": "acceptEdits" },
    });
    // With a persistent config directory (CLAUDE_CONFIG_DIR under the home) the settings
    // and the state, including a login the user made themselves, live on the persistent
    // mount and survive the sandbox being recreated. Otherwise the default locations.
    let dir = get(environment, "CLAUDE_CONFIG_DIR")
        .and_then(|path| path.strip_prefix("/home/nestbox/"))
        .map(str::to_string);
    let (settings_path, state_path) = match &dir {
        Some(dir) => (format!("{dir}/settings.json"), format!("{dir}/.claude.json")),
        None => (".claude/settings.json".to_string(), ".claude.json".to_string()),
    };
    vec![
        AgentFile {
            path: settings_path,
            content: serde_json::to_string_pretty(&settings).unwrap_or_default(),
            merge_json: false,
        },
        // Skip the first-run wizard: the endpoint and the credential are already set.
        AgentFile {
            path: state_path,
            content: serde_json::to_string_pretty(&claude_state(environment, servers)).unwrap_or_default(),
            merge_json: true,
        },
    ]
}

/// The URL an agent calls for this server.
///
/// The media server names the sandbox in the path, not only in a header: an
/// egress proxy may drop an unknown header, and a header-only design then fails
/// with "this connection does not name a sandbox" while everything else works.
/// A detached or unspecified sandbox keeps the plain endpoint.
fn endpoint_with_sandbox(server: &McpServerConfig) -> String {
    let base = server.endpoint.clone().unwrap_or_default();
    match (server.key.as_str(), server.sandbox.as_deref()) {
        ("nestbox-media", Some(sandbox)) if !sandbox.is_empty() => {
            format!("{}/{}", base.trim_end_matches('/'), sandbox)
        }
        _ => base,
    }
}

/// `~/.claude.json`: skip the wizard and register the Nestbox MCP hub when there is one.
/// The token is a `${NESTBOX_MCP_TOKEN}` reference, expanded by Claude Code from the
/// environment, never written to the file.
fn claude_state(environment: &BTreeMap<String, String>, servers: &[McpServerConfig]) -> serde_json::Value {
    let mut state = json!({ "hasCompletedOnboarding": true, "theme": "dark" });
    let mut configured = serde_json::Map::new();
    if let Some(url) = get(environment, "NESTBOX_MCP_URL") {
        configured.insert(
            "nestbox-hub".into(),
            json!({
                "type": "http",
                "url": url,
                "headers": { "Authorization": "Bearer ${NESTBOX_MCP_TOKEN}" },
            }),
        );
    }
    for server in servers {
        if server.key == "nestbox-hub" {
            continue;
        }
        let entry = match server.transport.as_str() {
            "stdio" => json!({
                "command": server.command.clone().unwrap_or_default(),
                "args": server.args,
            }),
            _ => {
                let mut headers = serde_json::Map::new();
                if let Some(variable) = &server.token_env {
                    headers.insert("Authorization".into(), json!(format!("Bearer ${{{variable}}}")));
                }
                if let Some(sandbox) = &server.sandbox {
                    headers.insert("X-Nestbox-Sandbox".into(), json!(sandbox));
                }
                json!({
                    "type": "http",
                    "url": endpoint_with_sandbox(server),
                    "headers": headers,
                })
            }
        };
        configured.insert(server.key.clone(), entry);
    }
    if !configured.is_empty() {
        state["mcpServers"] = serde_json::Value::Object(configured);
    }
    state
}

fn opencode(environment: &BTreeMap<String, String>, servers: &[McpServerConfig]) -> Vec<AgentFile> {
    // The models the sandbox may use, as the router knows them (contract C6). An
    // empty list keeps the older single-model shape, so an environment that was
    // written before grants existed still produces a working file.
    let granted: Vec<String> = get(environment, "NESTBOX_MODEL_LIST")
        .map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let models: Vec<String> = if granted.is_empty() {
        get(environment, "OPENAI_MODEL")
            .or_else(|| get(environment, "ANTHROPIC_MODEL"))
            .map(|model| vec![model.to_string()])
            .unwrap_or_default()
    } else {
        granted
    };
    // `mcp`, `autoupdate` and `share` belong to every sign-in mode; only the
    // provider below depends on it.
    let mut config = json!({
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": false,
        "share": "disabled",
    });
    match get(environment, "OPENAI_BASE_URL") {
        // The user signs in with their own account: the file is still written,
        // because it has to replace the one that pointed at the gateway, but
        // without a provider, a model or a small model.
        None => {}
        // An endpoint without any model name would be an empty provider that
        // shadows the agent's own defaults, so nothing is written (as before).
        Some(_) if models.is_empty() => return Vec::new(),
        Some(base) => {
            // A granted model wins over the profile's single default: the console
            // decided which one a new session should ask for.
            let default = get(environment, "NESTBOX_MODEL_DEFAULT")
                .filter(|name| models.iter().any(|candidate| candidate == name))
                .map(str::to_string)
                .or_else(|| models.first().cloned())
                .unwrap_or_default();
            let mut entries = serde_json::Map::new();
            for name in &models {
                entries.insert(name.clone(), json!({ "name": name }));
            }
            config["model"] = json!(format!("nestbox/{default}"));
            config["small_model"] = json!(format!("nestbox/{default}"));
            config["provider"] = json!({
                "nestbox": {
                    "npm": "@ai-sdk/openai-compatible",
                    "name": "Model endpoint (via the egress proxy)",
                    "options": {
                        "baseURL": base,
                        // A reference, not the value: the key lives in the environment.
                        "apiKey": "{env:OPENAI_API_KEY}",
                    },
                    "models": entries,
                }
            });
        }
    }
    let mut configured = serde_json::Map::new();
    if let Some(url) = get(environment, "NESTBOX_MCP_URL") {
        configured.insert(
            "nestbox-hub".into(),
            json!({
                "type": "remote",
                "url": url,
                "enabled": true,
                "headers": { "Authorization": "Bearer {env:NESTBOX_MCP_TOKEN}" },
            }),
        );
    }
    for server in servers {
        if server.key == "nestbox-hub" {
            continue;
        }
        let entry = match server.transport.as_str() {
            "stdio" => {
                let mut command = vec![server.command.clone().unwrap_or_default()];
                command.extend(server.args.iter().cloned());
                json!({ "type": "local", "command": command, "enabled": true })
            }
            _ => {
                let mut headers = serde_json::Map::new();
                if let Some(variable) = &server.token_env {
                    headers.insert("Authorization".into(), json!(format!("Bearer {{env:{variable}}}")));
                }
                if let Some(sandbox) = &server.sandbox {
                    headers.insert("X-Nestbox-Sandbox".into(), json!(sandbox));
                }
                json!({
                    "type": "remote",
                    "url": endpoint_with_sandbox(server),
                    "enabled": true,
                    "headers": headers,
                })
            }
        };
        configured.insert(server.key.clone(), entry);
    }
    if !configured.is_empty() {
        config["mcp"] = serde_json::Value::Object(configured);
    }
    vec![AgentFile {
        merge_json: false,
        path: ".config/opencode/opencode.json".into(),
        content: serde_json::to_string_pretty(&config).unwrap_or_default(),
    }]
}

/// The environment an agent that talks to OmniRoute (or any OpenAI/Anthropic
/// compatible gateway) is given: both API families, one model.
///
/// `endpoint` is the profile's base URL including `/v1`; Anthropic clients add
/// `/v1/messages` themselves, so their base has no `/v1`.
pub fn gateway_environment(endpoint: &str, model: Option<&str>, key: &str) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    let openai_base = endpoint.trim_end_matches('/').to_string();
    let anthropic_base = openai_base.strip_suffix("/v1").unwrap_or(&openai_base).to_string();
    environment.insert("OPENAI_BASE_URL".into(), openai_base.clone());
    environment.insert("OPENAI_API_BASE".into(), openai_base);
    environment.insert("OPENAI_API_KEY".into(), key.to_string());
    environment.insert("ANTHROPIC_BASE_URL".into(), anthropic_base);
    environment.insert("ANTHROPIC_AUTH_TOKEN".into(), key.to_string());
    if let Some(model) = model {
        environment.insert("ANTHROPIC_MODEL".into(), model.to_string());
        environment.insert("OPENAI_MODEL".into(), model.to_string());
        environment.insert("HERMES_MODEL".into(), model.to_string());
    }
    environment
}

/// Point the model base URLs of an agent environment at the accounting proxy.
///
/// The proxy needs two facts the request itself cannot carry through the egress
/// proxy (which drops `X-Nestbox-*` headers): which sandbox asks, and where the
/// request was meant to go. Both are put in the path, `/s/<sandbox>/u/<scheme>/
/// <host>/…`, and the proxy strips them again before it forwards. A base URL that
/// is not http(s) or that is already routed is left alone, so the call is safe to
/// repeat.
pub fn route_through_accounting(
    environment: &mut BTreeMap<String, String>,
    proxy_url: &str,
    sandbox_name: &str,
) {
    let proxy = proxy_url.trim().trim_end_matches('/');
    if proxy.is_empty() || sandbox_name.is_empty() {
        return;
    }
    // With the model router, the model is chosen per request by the accounting
    // proxy, so the base URLs name the sandbox and nothing else: no upstream host
    // is baked into the agent's environment, which is what lets a grant change a
    // model without a new container.
    if get(environment, "NESTBOX_MODEL_ROUTER") == Some("1") {
        for key in ["OPENAI_BASE_URL", "OPENAI_API_BASE"] {
            if environment.contains_key(key) {
                environment.insert(key.to_string(), format!("{proxy}/s/{sandbox_name}/r/v1"));
            }
        }
        if environment.contains_key("ANTHROPIC_BASE_URL") {
            environment.insert("ANTHROPIC_BASE_URL".to_string(), format!("{proxy}/s/{sandbox_name}/r"));
        }
        return;
    }
    for key in ["OPENAI_BASE_URL", "OPENAI_API_BASE", "ANTHROPIC_BASE_URL"] {
        let Some(current) = environment.get(key).cloned() else { continue };
        if current.starts_with(proxy) {
            continue;
        }
        let Some((scheme, rest)) = current.split_once("://") else { continue };
        if scheme != "http" && scheme != "https" {
            continue;
        }
        environment.insert(key.to_string(), format!("{proxy}/s/{sandbox_name}/u/{scheme}/{rest}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> BTreeMap<String, String> {
        let mut environment = gateway_environment("http://10.90.92.30:20128/v1/", Some("opencode-go-combo"), "sk-secret");
        environment.insert("HTTPS_PROXY".into(), "http://egress-proxy:3128".into());
        environment.insert("NO_PROXY".into(), "localhost,egress-proxy".into());
        environment
    }

    #[test]
    fn the_anthropic_base_has_no_v1_and_the_openai_base_keeps_it() {
        let environment = env();
        assert_eq!(environment["ANTHROPIC_BASE_URL"], "http://10.90.92.30:20128");
        assert_eq!(environment["OPENAI_BASE_URL"], "http://10.90.92.30:20128/v1");
    }

    #[test]
    fn claude_gets_its_endpoint_proxy_and_no_telemetry_but_never_the_key() {
        let files = files_for("claude-code", &env());
        let settings = files.iter().find(|file| file.path == ".claude/settings.json").expect("settings");
        assert!(settings.content.contains("ANTHROPIC_BASE_URL"));
        assert!(settings.content.contains("egress-proxy:3128"));
        assert!(settings.content.contains("opencode-go-combo"));
        assert!(settings.content.contains("DISABLE_TELEMETRY"));
        assert!(!settings.content.contains("sk-secret"), "the credential stays in the environment");
        let parsed: serde_json::Value = serde_json::from_str(&settings.content).expect("valid json");
        assert_eq!(parsed["env"]["ANTHROPIC_MODEL"], "opencode-go-combo");
        assert!(files.iter().any(|file| file.path == ".claude.json"));
    }

    #[test]
    fn a_default_granted_model_wins_over_the_profile_model() {
        let mut environment = env();
        environment.insert("NESTBOX_MODEL_DEFAULT".into(), "big-model".into());
        let files = files_for("claude-code", &environment);
        let settings = files.iter().find(|file| file.path == ".claude/settings.json").expect("settings");
        let parsed: serde_json::Value = serde_json::from_str(&settings.content).expect("valid json");
        assert_eq!(parsed["env"]["ANTHROPIC_MODEL"], "big-model");
        assert_eq!(parsed["env"]["ANTHROPIC_SMALL_FAST_MODEL"], "big-model");
    }

    #[test]
    fn opencode_and_openchamber_get_a_provider_that_refers_to_the_key_by_name() {
        for pack in ["opencode", "openchamber"] {
            let files = files_for(pack, &env());
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path, ".config/opencode/opencode.json");
            assert!(files[0].content.contains("{env:OPENAI_API_KEY}"));
            assert!(!files[0].content.contains("sk-secret"));
            let parsed: serde_json::Value = serde_json::from_str(&files[0].content).expect("valid json");
            assert_eq!(parsed["model"], "nestbox/opencode-go-combo");
        }
    }

    #[test]
    fn the_mcp_hub_is_registered_by_reference_never_with_the_token() {
        let mut environment = env();
        environment.insert("NESTBOX_MCP_URL".into(), "http://mcp-hub:8080/mcp".into());
        environment.insert("NESTBOX_MCP_TOKEN".into(), "tok-secret".into());
        let claude = files_for("claude-code", &environment);
        let state = claude.iter().find(|f| f.path == ".claude.json").expect("state");
        assert!(state.content.contains("http://mcp-hub:8080/mcp") && state.content.contains("${NESTBOX_MCP_TOKEN}"));
        assert!(!state.content.contains("tok-secret"));
        let code = files_for("opencode", &environment);
        assert!(code[0].content.contains("{env:NESTBOX_MCP_TOKEN}") && !code[0].content.contains("tok-secret"));
    }

    fn servers() -> Vec<McpServerConfig> {
        vec![
            McpServerConfig {
                key: "graphify".into(),
                transport: "stdio".into(),
                endpoint: None,
                command: Some("graphify".into()),
                args: vec!["--mcp".into(), "--graph".into(), "/home/nestbox/brain/graphify-out/graph.json".into()],
                token_env: None,
                sandbox: Some("nest".into()),
            },
            McpServerConfig {
                key: "nestbox-media".into(),
                transport: "http".into(),
                endpoint: Some("http://media-mcp:8080/mcp".into()),
                command: None,
                args: vec![],
                token_env: Some("NESTBOX_MEDIA_TOKEN".into()),
                sandbox: Some("nest".into()),
            },
        ]
    }

    #[test]
    fn an_attached_graphify_server_is_written_as_a_stdio_entry() {
        let files = files_for_with_servers("claude-code", &env(), &servers());
        let state = files.iter().find(|f| f.path == ".claude.json").expect("state");
        let parsed: serde_json::Value = serde_json::from_str(&state.content).expect("valid json");
        assert_eq!(parsed["mcpServers"]["graphify"]["command"], "graphify");
        assert_eq!(parsed["mcpServers"]["graphify"]["args"][0], "--mcp");
    }

    #[test]
    fn the_media_server_carries_the_sandbox_name_and_a_token_reference_only() {
        let files = files_for_with_servers("opencode", &env(), &servers());
        let parsed: serde_json::Value = serde_json::from_str(&files[0].content).expect("valid json");
        let media = &parsed["mcp"]["nestbox-media"];
        assert_eq!(media["url"], "http://media-mcp:8080/mcp/nest");
        assert_eq!(media["headers"]["Authorization"], "Bearer {env:NESTBOX_MEDIA_TOKEN}");
        assert_eq!(media["headers"]["X-Nestbox-Sandbox"], "nest");
        assert!(!files[0].content.contains("tok-secret"));
    }

    #[test]
    fn a_detached_server_is_absent_from_the_next_config() {
        let files = files_for_with_servers("opencode", &env(), &[]);
        let parsed: serde_json::Value = serde_json::from_str(&files[0].content).expect("valid json");
        assert!(parsed.get("mcp").is_none());
    }

    #[test]
    fn a_pack_without_configuration_gets_no_files() {
        assert!(files_for("ubuntu-dev", &env()).is_empty());
    }

    #[test]
    fn opencode_without_an_endpoint_has_no_provider_but_keeps_the_rest() {
        // Own login: the file must still replace the one that named the gateway,
        // otherwise a re-apply would leave the user pointed at a credential the
        // launcher just stripped.
        let files = files_for("opencode", &BTreeMap::new());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, ".config/opencode/opencode.json");
        let parsed: serde_json::Value = serde_json::from_str(&files[0].content).expect("valid json");
        assert!(parsed.get("provider").is_none());
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("small_model").is_none());
        assert_eq!(parsed["autoupdate"], false);
        assert_eq!(parsed["share"], "disabled");
    }

    #[test]
    fn opencode_lists_every_granted_model_and_takes_the_default_one() {
        let mut environment = env();
        environment.insert("NESTBOX_MODEL_LIST".into(), "a,b".into());
        environment.insert("NESTBOX_MODEL_DEFAULT".into(), "b".into());
        let files = files_for("opencode", &environment);
        let parsed: serde_json::Value = serde_json::from_str(&files[0].content).expect("valid json");
        assert_eq!(parsed["model"], "nestbox/b");
        assert_eq!(parsed["small_model"], "nestbox/b");
        assert!(parsed["provider"]["nestbox"]["models"].get("a").is_some());
        assert!(parsed["provider"]["nestbox"]["models"].get("b").is_some());
    }

    #[test]
    fn opencode_falls_back_to_the_first_granted_model() {
        let mut environment = env();
        environment.insert("NESTBOX_MODEL_LIST".into(), "a,b".into());
        let parsed: serde_json::Value =
            serde_json::from_str(&files_for("opencode", &environment)[0].content).expect("valid json");
        assert_eq!(parsed["model"], "nestbox/a");
    }

    #[test]
    fn in_router_mode_the_base_urls_name_the_sandbox_and_nothing_else() {
        let mut environment = gateway_environment("http://10.90.92.30:20128/v1", Some("m"), "k");
        environment.insert("NESTBOX_MODEL_ROUTER".into(), "1".into());
        route_through_accounting(&mut environment, "http://accounting-proxy:8099/", "x");
        assert_eq!(environment["OPENAI_BASE_URL"], "http://accounting-proxy:8099/s/x/r/v1");
        assert_eq!(environment["OPENAI_API_BASE"], "http://accounting-proxy:8099/s/x/r/v1");
        assert_eq!(environment["ANTHROPIC_BASE_URL"], "http://accounting-proxy:8099/s/x/r");
        assert_eq!(environment["OPENAI_API_KEY"], "k", "the credential is untouched");
    }

    #[test]
    fn model_base_urls_are_routed_through_the_accounting_proxy_by_path() {
        let mut environment = gateway_environment("http://10.90.92.30:20128/v1", Some("m"), "k");
        route_through_accounting(&mut environment, "http://host.openshell.internal:18099/", "claude-01");
        assert_eq!(
            environment["ANTHROPIC_BASE_URL"],
            "http://host.openshell.internal:18099/s/claude-01/u/http/10.90.92.30:20128"
        );
        assert_eq!(
            environment["OPENAI_BASE_URL"],
            "http://host.openshell.internal:18099/s/claude-01/u/http/10.90.92.30:20128/v1"
        );
        assert_eq!(environment["OPENAI_API_KEY"], "k", "the credential is untouched");
        let once = environment.clone();
        route_through_accounting(&mut environment, "http://host.openshell.internal:18099", "claude-01");
        assert_eq!(environment, once, "routing twice changes nothing");
    }
}
