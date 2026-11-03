//! The builder: a recipe is the desired state of a sandbox image.
//!
//! A recipe names a baseline, tools, agents, resources, and mounts. Building it
//! produces an immutable image tagged by the content hash of the recipe, so the
//! same recipe always yields the same image and an edit always yields a new
//! one. Persistent data never lives in the image.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Whether agents in a sandbox share one home or each get their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HomeMode {
    /// `/home/nestbox`, shared by every agent in the sandbox.
    Shared,
    /// `/home/agents/<instance>`, one per agent.
    Isolated,
}

impl HomeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Isolated => "isolated",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "shared" => Self::Shared,
            "isolated" => Self::Isolated,
            _ => return None,
        })
    }

    /// The path inside the container for an agent instance.
    pub fn home_path(self, instance: &str) -> String {
        match self {
            Self::Shared => "/home/nestbox".to_string(),
            Self::Isolated => format!("/home/agents/{instance}"),
        }
    }
}

/// The instance name the supervisor runs under.
///
/// It is a fixed word, not an operator choice: the control plane delivers
/// settings and commands into this instance's home, and a name the operator
/// could change would break that path silently.
pub const SUPERVISOR_INSTANCE: &str = "supervisor";

/// Where the supervisor keeps its state directory.
///
/// Both sides must agree: the process writes its reports from here, and the
/// control plane writes the settings and commands it reads. The sandbox exports
/// the same value as `NESTBOX_SUPERVISOR_STATE`.
pub fn supervisor_state_directory() -> String {
    format!(
        "{}/.nestbox-supervisor",
        HomeMode::Isolated.home_path(SUPERVISOR_INSTANCE)
    )
}

/// Whether an agent's home survives the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Persistence {
    /// The home is a persistent root on the node.
    Persistent,
    /// The home lives in the container and dies with it.
    Ephemeral,
}

impl Persistence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::Ephemeral => "ephemeral",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "persistent" => Self::Persistent,
            "ephemeral" => Self::Ephemeral,
            _ => return None,
        })
    }
}

/// One agent inside a recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AgentSpec {
    pub pack_key: String,
    /// Instance name, unique within the recipe: `hermes`, `claude`.
    pub instance: String,
    pub home_mode: HomeMode,
    pub persistence: Persistence,
    /// Overrides the pack's `home_paths` when non-empty.
    #[serde(default)]
    pub home_paths: Vec<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

/// The limits a recipe asks for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ResourceLimits {
    /// Docker-style string, normalised to bytes by the sandbox service.
    #[serde(default = "default_memory")]
    pub memory_limit: String,
    #[serde(default = "default_cpu")]
    pub cpu_limit: f64,
    #[serde(default)]
    pub gpu_count: u32,
    #[serde(default = "default_pids")]
    pub pids_limit: i64,
}

fn default_memory() -> String {
    "1g".into()
}
fn default_cpu() -> f64 {
    1.0
}
fn default_pids() -> i64 {
    256
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_limit: default_memory(),
            cpu_limit: default_cpu(),
            gpu_count: 0,
            pids_limit: default_pids(),
        }
    }
}

/// A project root mounted into the sandbox a recipe produces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProjectMount {
    pub root_id: Uuid,
    pub mount_path: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecipeState {
    Draft,
    Building,
    Built,
    Failed,
}

impl RecipeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Building => "building",
            Self::Built => "built",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "draft" => Self::Draft,
            "building" => Self::Building,
            "built" => Self::Built,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// A recipe as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Recipe {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub owner_user_id: Option<Uuid>,
    pub name: String,
    pub baseline_key: String,
    pub tool_profile_id: Option<Uuid>,
    pub tools: Vec<String>,
    /// Extra package names selected from the baseline's native package catalogue.
    pub extra_packages: Vec<String>,
    /// Public repository definitions; credentials are vault locators, never values.
    pub repositories: Vec<RepositorySpec>,
    /// Registry MCP servers attached when an image becomes a sandbox.
    pub mcp_server_keys: Vec<String>,
    pub agents: Vec<AgentSpec>,
    pub resources: ResourceLimits,
    pub project_mounts: Vec<ProjectMount>,
    pub node_id: Uuid,
    pub state: RecipeState,
    pub image_ref: Option<String>,
    pub content_hash: Option<String>,
    pub revision: i64,
    pub build_job_id: Option<Uuid>,
    pub build_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A saved builder selection that can be reused by other recipes.
///
/// It carries catalog keys and package names, never an image: applying a
/// template to a recipe copies these fields, and the recipe's content hash then
/// decides whether the next build produces a new image.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageTemplate {
    pub id: Uuid,
    pub environment_id: Uuid,
    pub name: String,
    pub description: String,
    pub baseline_key: String,
    pub tools: Vec<String>,
    pub extra_packages: Vec<String>,
    pub repositories: Vec<RepositorySpec>,
    pub mcp_server_keys: Vec<String>,
    /// The one selection a new recipe starts from; at most one per environment.
    pub is_default: bool,
    /// Offered to the environment's other operators.
    pub is_shared: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A signed repository registered before an image installs optional packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RepositorySpec {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub suite: Option<String>,
    #[serde(default)]
    pub components: Vec<String>,
    /// Public ASCII-armoured signing key. Private credentials use a vault locator.
    #[serde(default)]
    pub signing_key: Option<String>,
    #[serde(default)]
    pub credential_ref: Option<String>,
}

/// A valid recipe name: lowercase slug, so it can become an image tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeName(String);

impl RecipeName {
    /// Validate a recipe name.
    pub fn new(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.is_empty() || value.len() > 63 {
            return Err("A recipe name is 1 to 63 characters.".into());
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || value.starts_with('-')
        {
            return Err("A recipe name uses lowercase letters, digits, and hyphens.".into());
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_name_is_a_slug() {
        assert!(RecipeName::new("dev-box").is_ok());
        assert!(RecipeName::new("Dev Box").is_err());
        assert!(RecipeName::new("-x").is_err());
        assert!(RecipeName::new("").is_err());
    }

    #[test]
    fn home_mode_decides_the_path() {
        assert_eq!(HomeMode::Shared.home_path("hermes"), "/home/nestbox");
        assert_eq!(HomeMode::Isolated.home_path("hermes"), "/home/agents/hermes");
    }

    #[test]
    fn resource_limits_have_sane_defaults() {
        let r = ResourceLimits::default();
        assert_eq!(r.memory_limit, "1g");
        assert_eq!(r.gpu_count, 0);
    }
}


// ---------------------------------------------------------------------------
// The supervisor script
// ---------------------------------------------------------------------------

/// One agent as the entrypoint generator needs it.
///
/// Owned rather than borrowed so a caller can collect launches while it walks
/// the packs; the generator itself is a pure function and does not care where
/// the values came from.
pub struct AgentLaunch {
    pub instance: String,
    pub home_mode: HomeMode,
    pub command: Vec<String>,
    pub env_slots: Vec<String>,
    pub port: Option<u16>,
}

/// The directory where every supervised process writes its PID file.
///
/// The convention is shared with the control plane: `nestbox-agentctl restart
/// <name>` and the supervisors of other images find a process by this path, so
/// the generator must not invent its own location.
const AGENT_PID_DIRECTORY: &str = "/tmp/nestbox-agents";

/// Render the container command that starts every agent in a sandbox.
///
/// One supervisor script, not one process per container: each agent runs in a
/// subshell that changes into its home, exports the pack's environment slots,
/// and is supervised there. One agent per home; an agent that stops is started
/// again after 2 s, so a crash or a sign-in mode change costs one process and
/// not the whole sandbox.
///
/// Each agent's subshell writes the PID of the process it just started to
/// `/tmp/nestbox-agents/<name>.pid`, which is how `nestbox-agentctl restart`
/// and the mode change reach exactly one agent. The script sources
/// `.nestbox/env.sh` before every start, so an edited environment is picked up
/// by the next start without restarting the container.
///
/// bash is used rather than `/bin/sh` for `wait $!` and because the pack
/// commands assume it; every sandbox image starts from Ubuntu or Debian and
/// ships bash 5.
///
/// A pack without a command (it is its own image with its own entrypoint) gets
/// `sleep infinity`, so the supervisor still owns one PID per agent and the
/// health of the sandbox stays tied to the agents it claims to run.
pub fn entrypoint_command(agents: &[AgentLaunch]) -> Vec<String> {
    let mut script = String::from(
        "set -u\n\
         # Nestbox sandbox supervisor: one agent per home; an agent that stops\n\
         # is started again after 2 s.\n",
    );

    if agents.is_empty() {
        script.push_str(
            "# No agent is configured, so the container stays idle until it is\n\
             # asked to do something with an exec.\n\
             exec sleep infinity\n",
        );
        return vec!["/bin/bash".to_string(), "-c".to_string(), script];
    }

    script.push_str(&format!("\nmkdir -p {AGENT_PID_DIRECTORY}\n"));

    for agent in agents {
        let home = agent.home_mode.home_path(&agent.instance);
        let quoted = quote(&home);
        let pidfile = quote(&format!("{AGENT_PID_DIRECTORY}/{}.pid", agent.instance));
        script.push_str(&format!(
            "\necho \"nestbox: starting {0} in {1}\"\n# agent {0} in {1} ({2})\nmkdir -p {quoted}\n(\n  cd {quoted} || exit 1\n  export HOME={quoted}\n",
            agent.instance,
            home,
            if agent.home_mode == HomeMode::Shared {
                "shared home"
            } else {
                "isolated home"
            },
        ));
        for slot in agent.env_slots.iter().filter(|slot| is_env_name(slot)) {
            // Exported even when unset, so the agent reads an empty value
            // instead of inheriting whatever the image happens to define.
            script.push_str(&format!("  export {slot}=\"${{{slot}:-}}\"\n"));
        }
        if let Some(port) = agent.port {
            script.push_str(&format!("  export NESTBOX_AGENT_PORT={port}\n"));
        }
        let command = if agent.command.is_empty() {
            vec!["sleep".to_string(), "infinity".to_string()]
        } else {
            agent.command.to_vec()
        };
        let line: Vec<String> = command.iter().map(|part| quote(part)).collect();
        script.push_str(&format!(
            "  while true; do\n    [ -f \"$HOME/.nestbox/env.sh\" ] && . \"$HOME/.nestbox/env.sh\"\n    {0} &\n    echo $! > {pidfile}\n    wait $!\n    echo \"nestbox: {1} exited with status $?; starting it again in 2 s\" >&2\n    sleep 2\n  done\n) &\n",
            line.join(" "),
            agent.instance,
        ));
    }

    script.push_str(
        "\necho \"nestbox: supervisor up\"\n\n# Every agent runs in its own loop, so one agent stopping is not the\n\
         # end of the sandbox; the supervisor stays until the container is\n\
         # stopped, and `wait` without arguments keeps it in the foreground.\n\
         wait\n",
    );
    vec!["/bin/bash".to_string(), "-c".to_string(), script]
}

/// Whether an environment slot name may appear in a generated script.
///
/// Slot names come from the catalog, which is data. A name that is not a shell
/// identifier would turn into a syntax error — or into shell code — in a
/// generated script, so it is dropped instead.
fn is_env_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_uppercase() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Quote one argument for the supervisor script.
///
/// Only the characters that are safe unquoted stay unquoted; everything else is
/// wrapped in single quotes, which is the one form the shell does not interpret.
fn quote(value: &str) -> String {
    let safe = !value.is_empty()
        && value.chars().all(|letter| {
            letter.is_ascii_alphanumeric() || matches!(letter, '_' | '-' | '.' | '/' | ':' | '@' | '=')
        });
    if safe {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod supervisor_tests {
    use super::*;

    fn launch(
        instance: &str,
        home_mode: HomeMode,
        command: &[String],
        env_slots: &[String],
        port: Option<u16>,
    ) -> AgentLaunch {
        AgentLaunch {
            instance: instance.to_string(),
            home_mode,
            command: command.to_vec(),
            env_slots: env_slots.to_vec(),
            port,
        }
    }

    #[test]
    fn two_agents_with_different_homes_get_one_supervisor() {
        let hermes_command = vec!["sleep".to_string(), "infinity".to_string()];
        let hermes_env = vec!["OPENAI_BASE_URL".to_string(), "OPENAI_API_KEY".to_string()];
        let claude_command = vec!["claude".to_string(), "--dangerously-skip-permissions".to_string()];
        let claude_env = vec!["ANTHROPIC_API_KEY".to_string()];

        let agents = vec![
            launch("hermes", HomeMode::Shared, &hermes_command, &hermes_env, None),
            launch("claude", HomeMode::Isolated, &claude_command, &claude_env, Some(3000)),
        ];
        let command = entrypoint_command(&agents);

        assert_eq!(command.len(), 3);
        assert_eq!(command[0], "/bin/bash");
        assert_eq!(command[1], "-c");
        let script = &command[2];

        // One supervisor, one script, and no `wait -n`: an agent that stops is
        // restarted by its own loop instead of ending the sandbox.
        assert!(!script.contains("wait -n"));
        assert!(script.contains("while true; do"));
        assert_eq!(script.matches("while true; do").count(), 2);
        assert!(script.trim_end().ends_with("wait"));

        // The PID directory exists before the first agent writes into it.
        assert!(script.contains("mkdir -p /tmp/nestbox-agents"));
        assert!(script.contains("echo $! > /tmp/nestbox-agents/claude.pid"));

        // The shared agent lands in /home/nestbox, the isolated one in its own.
        assert!(script.contains("mkdir -p /home/nestbox"));
        assert!(script.contains("cd /home/nestbox || exit 1"));
        assert!(script.contains("mkdir -p /home/agents/claude"));
        assert!(script.contains("cd /home/agents/claude || exit 1"));
        assert!(!script.contains("/home/agents/hermes"));

        // Each agent is started, and its own slots are exported for it only.
        assert!(script.contains("    sleep infinity &"));
        assert!(script.contains("    claude --dangerously-skip-permissions &"));
        assert!(script.contains("export OPENAI_BASE_URL=\"${OPENAI_BASE_URL:-}\""));
        assert!(script.contains("export OPENAI_API_KEY=\"${OPENAI_API_KEY:-}\""));
        assert!(script.contains("export ANTHROPIC_API_KEY=\"${ANTHROPIC_API_KEY:-}\""));
        assert!(script.contains("export NESTBOX_AGENT_PORT=3000"));

        // One background subshell per agent.
        assert_eq!(script.matches(") &").count(), 2);

        // Every start reads the environment the control plane wrote.
        assert_eq!(script.matches("[ -f \"$HOME/.nestbox/env.sh\" ]").count(), 2);

        // The claude block must not carry hermes's slots.
        let claude_block = script.split("# agent claude").nth(1).unwrap_or_default();
        assert!(!claude_block.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn an_agent_without_a_command_keeps_its_slot() {
        let empty: Vec<String> = Vec::new();
        let slots: Vec<String> = Vec::new();
        let command = entrypoint_command(&[launch(
            "openclaw",
            HomeMode::Isolated,
            &empty,
            &slots,
            None,
        )]);
        assert!(command[2].contains("    sleep infinity &"));
        assert_eq!(command[2].matches(") &").count(), 1);
    }

    #[test]
    fn a_sandbox_without_agents_stays_idle() {
        let command = entrypoint_command(&[]);
        assert_eq!(command[0], "/bin/bash");
        assert!(command[2].contains("exec sleep infinity"));
        assert!(!command[2].contains("wait -n"));
        assert!(!command[2].contains("while true; do"));
    }

    #[test]
    fn a_hostile_command_is_quoted() {
        let command = vec!["sh".to_string(), "-c".to_string(), "echo 'hi there'".to_string()];
        let slots: Vec<String> = Vec::new();
        let rendered = entrypoint_command(&[launch("x", HomeMode::Shared, &command, &slots, None)]);
        assert!(rendered[2].contains("sh -c 'echo '\\''hi there'\\''' &"));
    }

    #[test]
    fn a_hostile_instance_does_not_escape_the_pid_path() {
        let empty: Vec<String> = Vec::new();
        let slots: Vec<String> = Vec::new();
        let rendered =
            entrypoint_command(&[launch("a; id", HomeMode::Isolated, &empty, &slots, None)]);
        assert!(!rendered[2].contains("> /tmp/nestbox-agents/a; id.pid"));
        assert!(rendered[2].contains("/tmp/nestbox-agents/a; id.pid'"));
    }

    #[test]
    fn the_generated_script_parses() {
        // The script runs in a container where a syntax error would cost the
        // whole sandbox, so the generator is checked with a real shell parser.
        let command = vec!["claude".to_string(), "--continue".to_string()];
        let slots = vec!["ANTHROPIC_API_KEY".to_string()];
        let agents = vec![
            launch("hermes", HomeMode::Shared, &command, &slots, None),
            launch("claude", HomeMode::Isolated, &command, &slots, Some(3000)),
        ];
        for agent_set in [Vec::new(), agents] {
            let rendered = entrypoint_command(&agent_set);
            let output = std::process::Command::new(&rendered[0])
                .arg("-n")
                .arg("-c")
                .arg(&rendered[2])
                .output();
            let Ok(output) = output else {
                // No bash on this machine: nothing to check against.
                return;
            };
            assert!(
                output.status.success(),
                "generated script does not parse: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn an_env_slot_that_is_not_a_name_is_dropped() {
        assert!(is_env_name("OPENAI_API_KEY"));
        assert!(is_env_name("_X1"));
        assert!(!is_env_name("openai"));
        assert!(!is_env_name("A B"));
        assert!(!is_env_name("A;rm -rf /"));

        let slots = vec!["A;rm -rf /".to_string(), "GOOD_ONE".to_string()];
        let command = vec!["sleep".to_string(), "infinity".to_string()];
        let rendered = entrypoint_command(&[launch("x", HomeMode::Shared, &command, &slots, None)]);
        assert!(!rendered[2].contains("rm -rf"));
        assert!(rendered[2].contains("export GOOD_ONE="));
    }
    #[test]
    fn the_supervisor_state_lives_in_the_supervisors_own_home() {
        // The control plane writes into this path and the process reads from
        // it; if the two ever disagree, settings and commands are delivered to
        // a directory nobody reads.
        assert_eq!(
            supervisor_state_directory(),
            format!("/home/agents/{SUPERVISOR_INSTANCE}/.nestbox-supervisor")
        );
    }

}
