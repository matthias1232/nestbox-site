//! Agent sign-in modes: which agents a sandbox has, whether each uses the
//! user's own subscription or the gateway, and what that means for config files
//! and firewall rules.
//!
//! The mode is a launcher's decision (ADR 0054): a small script in the sandbox
//! reads the mode file and either strips the gateway environment or keeps it.
//! Nothing here performs I/O, so the rules are provable in a unit test.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An agent whose sign-in can be switched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// Claude Code.
    Claude,
    /// OpenAI Codex CLI.
    Codex,
    /// OpenCode (also inside OpenChamber).
    Opencode,
}

impl AgentKind {
    /// Every switchable agent, in display order.
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode];

    /// The stored and wire form, equal to the binary name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }

    /// Parse the stored form.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "opencode" => Some(Self::Opencode),
            _ => None,
        }
    }

    /// Arguments that start the agent again where it stopped.
    pub fn resume_args(self) -> &'static str {
        match self {
            Self::Claude => "--continue",
            Self::Codex => "resume --last",
            Self::Opencode => "--continue",
        }
    }
}

/// How an agent signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentAuthMode {
    /// The user's own login (Claude Pro/Max, ChatGPT, ...), kept in the agent's home.
    Subscription,
    /// Models through the Nestbox gateway (OpenShell provider / OmniRoute).
    Gateway,
}

impl AgentAuthMode {
    /// The stored and wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Gateway => "gateway",
        }
    }

    /// Parse the stored form.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "subscription" => Some(Self::Subscription),
            "gateway" => Some(Self::Gateway),
            _ => None,
        }
    }
}

/// When running agents are restarted after a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Restart idle sessions now and busy ones as soon as they are idle.
    Idle,
    /// Restart every session now, busy or not.
    Now,
    /// Restart nothing now; mark the agent as waiting for a restart.
    Later,
}

impl RestartPolicy {
    /// The wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Now => "now",
            Self::Later => "later",
        }
    }

    /// Parse the wire form.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "idle" => Some(Self::Idle),
            "now" => Some(Self::Now),
            "later" => Some(Self::Later),
            _ => None,
        }
    }
}

/// One interactive agent session: a tmux pane running an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AgentSession {
    /// Which agent runs in the pane.
    pub agent: AgentKind,
    /// The tmux session name, e.g. `nestbox-t-abc`.
    pub tmux_session: String,
    /// The tmux pane id, e.g. `%3`.
    pub pane_id: String,
    /// The pane's process id.
    pub pid: i64,
    /// The pane's working directory.
    pub path: String,
    /// Seconds since the pane last printed something.
    pub idle_seconds: i64,
    /// True when the pane printed something in the last 20 seconds.
    pub busy: bool,
}

/// What the console shows for one agent of one sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentRuntimeState {
    /// The agent.
    pub agent: AgentKind,
    /// The chosen sign-in mode.
    pub mode: AgentAuthMode,
    /// True while a restart waits for the agent to become idle.
    pub pending_restart: bool,
    /// True when any session of this agent is busy.
    pub busy: bool,
    /// The interactive sessions found.
    pub sessions: Vec<AgentSession>,
    /// Whether an own login was found in the agent's home (`None` = unknown).
    pub signed_in: Option<bool>,
    /// The account label (e-mail) when known.
    pub account: Option<String>,
}

/// The hosts an agent's own login needs (port 443).
pub fn login_hosts(agent: AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::Claude => &[
            "claude.ai",
            "console.anthropic.com",
            "platform.claude.com",
            "api.anthropic.com",
            "statsig.anthropic.com",
        ],
        AgentKind::Codex => &["auth.openai.com", "chatgpt.com", "api.openai.com", "ab.chatgpt.com"],
        AgentKind::Opencode => &[
            "opencode.ai",
            "models.dev",
            "api.anthropic.com",
            "console.anthropic.com",
            "claude.ai",
            "api.openai.com",
            "auth.openai.com",
        ],
    }
}

/// Remove the gateway settings an agent in subscription mode must not see from
/// the map that feeds its config files.
///
/// The container environment is never changed by this: an agent started by the
/// launcher strips the variables itself, so switching needs no new container.
pub fn strip_for_modes(env: &mut BTreeMap<String, String>, modes: &[(AgentKind, AgentAuthMode)]) {
    for (agent, mode) in modes {
        if *mode != AgentAuthMode::Subscription {
            continue;
        }
        let keys: &[&str] = match agent {
            AgentKind::Claude => &[
                "ANTHROPIC_AUTH_TOKEN",
                "ANTHROPIC_API_KEY",
                "ANTHROPIC_BASE_URL",
                "ANTHROPIC_MODEL",
                "ANTHROPIC_SMALL_FAST_MODEL",
            ],
            AgentKind::Opencode => &[
                "OPENAI_BASE_URL",
                "OPENAI_API_BASE",
                "OPENAI_API_KEY",
                "OPENAI_MODEL",
            ],
            AgentKind::Codex => &[],
        };
        for key in keys {
            env.remove(*key);
        }
    }
}

/// The mode an agent has before anyone chose one: the sandbox's old create-time
/// login choice, so an existing sandbox keeps the behaviour it was made with.
pub fn default_mode(login: Option<&str>) -> AgentAuthMode {
    if login == Some("own_account") {
        AgentAuthMode::Subscription
    } else {
        AgentAuthMode::Gateway
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for kind in AgentKind::ALL {
            assert_eq!(AgentKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(AgentKind::parse("hermes"), None);
    }

    #[test]
    fn subscription_strips_only_its_own_agent() {
        let mut env = BTreeMap::new();
        env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), "k".to_string());
        env.insert("OPENAI_API_KEY".to_string(), "k".to_string());
        strip_for_modes(
            &mut env,
            &[
                (AgentKind::Claude, AgentAuthMode::Subscription),
                (AgentKind::Opencode, AgentAuthMode::Gateway),
            ],
        );
        assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
        assert!(env.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn old_own_account_choice_means_subscription() {
        assert_eq!(default_mode(Some("own_account")), AgentAuthMode::Subscription);
        assert_eq!(default_mode(Some("gateway")), AgentAuthMode::Gateway);
        assert_eq!(default_mode(None), AgentAuthMode::Gateway);
    }

    #[test]
    fn every_agent_has_login_hosts() {
        for kind in AgentKind::ALL {
            assert!(!login_hosts(kind).is_empty(), "{}", kind.as_str());
        }
    }
}
