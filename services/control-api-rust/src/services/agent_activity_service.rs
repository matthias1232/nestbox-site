//! What the agents in a sandbox are doing right now: their tmux panes, whether a
//! pane is in the middle of an answer, and how to restart one without losing the
//! conversation (ADR 0054).
//!
//! Filled by package B03.

use uuid::Uuid;

use super::ServiceContext;
use crate::domain::agent_auth::{AgentKind, AgentSession};
use crate::error::{ApiError, ErrorCode};
use crate::store::TenantScope;
use crate::store::postgres::agent_activity as activity_store;

/// Seconds without output after which a pane counts as idle.
const BUSY_WINDOW_SECONDS: i64 = 20;

/// How long the pane listing may take before the runtime call is given up.
const PANES_TIMEOUT_SECONDS: u32 = 10;

/// How long a restart may take before the runtime call is given up.
const RESTART_TIMEOUT_SECONDS: u32 = 10;

/// One line per tmux pane, with the pane's command line and its direct children.
///
/// Written as a shell script because the runtime only runs argument vectors; it
/// reads `/proc` and `awk` only, since the agent images carry no `ps`.
///
/// The line is emitted with tab separators, which is what `parse_panes` reads.
/// tmux 3.4 rewrites any control character in a `-F` format to `_`, so the pane
/// fields are read with `|` from tmux and re-emitted with tabs by `printf`.
const PANES: &str = r#"command -v tmux >/dev/null 2>&1 || exit 0
now="$(date +%s)"
tmux list-panes -a -F '#{session_name}|#{pane_id}|#{pane_pid}|#{pane_current_path}|#{window_activity}' 2>/dev/null |
while IFS='|' read -r session pane pid path activity; do
  cmd="$(tr '\0' ' ' < /proc/$pid/cmdline 2>/dev/null)"
  for c in /proc/[0-9]*; do
    ppid="$(awk '{print $4}' "$c/stat" 2>/dev/null)"
    [ "$ppid" = "$pid" ] && cmd="$cmd | $(tr '\0' ' ' < "$c/cmdline" 2>/dev/null)"
  done
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$session" "$pane" "$pid" "$path" "$activity" "$now" "$cmd"
done
"#;

/// The agent sessions in a sandbox, from its tmux panes.
///
/// A pane without an agent is not a session, so a plain shell pane never reaches
/// the console. A stopped sandbox answers through the runtime error, not with an
/// empty list, because "no agents" and "cannot look" are different answers.
pub async fn sessions(
    ctx: &ServiceContext,
    sandbox_name: &str,
) -> Result<Vec<AgentSession>, ApiError> {
    let command = vec!["sh".to_string(), "-c".to_string(), PANES.to_string()];
    match ctx.runtime.exec(sandbox_name, &command, PANES_TIMEOUT_SECONDS).await {
        Ok(result) if result.exit_code == Some(0) => Ok(parse_panes(&result.output)),
        Ok(result) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("The agent panes could not be listed: exit {:?}", result.exit_code),
        )),
        Err(error) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            error.to_string(),
        )),
    }
}

/// Whether the sandbox talked to a model in the last 30 seconds.
///
/// A supervised agent (OpenChamber, a recipe agent) has no pane activity to read,
/// so its busy state comes from the telemetry the sandbox already produces.
pub async fn recent_model_traffic(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
) -> Result<bool, ApiError> {
    let mut scope = TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let seen = activity_store::recent_model_traffic(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(seen)
}

/// Restart one pane, starting the agent again where it stopped.
///
/// The pane is respawned in its own directory with the launcher and the resume
/// argument of its agent; the shell that follows keeps the window useful if the
/// agent exits. The pane id and path are bound as `$1` and `$2`, so nothing from
/// the listing can become shell syntax.
pub async fn restart_session(
    ctx: &ServiceContext,
    sandbox_name: &str,
    session: &AgentSession,
) -> Result<(), ApiError> {
    let inner = format!(
        "\"$HOME/.nestbox/bin/{}\" {}; exec bash -l",
        session.agent.as_str(),
        session.agent.resume_args()
    );
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "tmux respawn-pane -k -t \"$1\" -c \"$2\" \"$3\"".to_string(),
        "sh".to_string(),
        session.pane_id.clone(),
        session.path.clone(),
        inner,
    ];
    match ctx
        .runtime
        .exec(sandbox_name, &command, RESTART_TIMEOUT_SECONDS)
        .await
    {
        Ok(result) if result.exit_code == Some(0) => Ok(()),
        Ok(result) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("The pane could not be restarted: {}", result.output.trim()),
        )),
        Err(error) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            error.to_string(),
        )),
    }
}

/// Restart a supervised agent (OpenChamber, a recipe agent) by its name.
///
/// The name comes from a request, so it is checked against the shape the agent
/// supervisor writes before it reaches the shell, and it still travels as an
/// argument rather than as part of the script.
pub async fn restart_supervised(
    ctx: &ServiceContext,
    sandbox_name: &str,
    name: &str,
) -> Result<(), ApiError> {
    if !valid_agent_name(name) {
        return Err(ApiError::validation(
            "An agent name is 1 to 64 letters, digits, underscores or dashes.",
        ));
    }
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "\"$HOME/.nestbox/bin/nestbox-agentctl\" restart \"$1\"".to_string(),
        "sh".to_string(),
        name.to_string(),
    ];
    match ctx
        .runtime
        .exec(sandbox_name, &command, RESTART_TIMEOUT_SECONDS)
        .await
    {
        Ok(result) if result.exit_code == Some(0) => Ok(()),
        Ok(result) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!("The agent could not be restarted: {}", result.output.trim()),
        )),
        Err(error) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            error.to_string(),
        )),
    }
}

/// Whether a name is one the agent supervisor could have created.
///
/// A closed character set rather than a regex, so the check cannot drift from
/// what the launcher and `nestbox-agentctl` accept.
fn valid_agent_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Turn the script output into sessions; lines without an agent are skipped.
///
/// One line is `session, pane, pid, path, activity, now, command`, separated by
/// tabs. The command field is last and may contain spaces and `|`, so the line is
/// split at most six times.
fn parse_panes(output: &str) -> Vec<AgentSession> {
    let mut sessions = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.splitn(7, '\t').collect();
        if fields.len() < 7 {
            continue;
        }
        let Some(agent) = agent_in(fields[6]) else {
            continue;
        };
        let pid = fields[2].trim().parse::<i64>().unwrap_or_else(|_| 0);
        let activity = fields[4].trim().parse::<i64>().unwrap_or_else(|_| 0);
        let now = fields[5].trim().parse::<i64>().unwrap_or_else(|_| 0);
        let idle_seconds = (now - activity).max(0);
        sessions.push(AgentSession {
            agent,
            tmux_session: fields[0].to_string(),
            pane_id: fields[1].to_string(),
            pid,
            path: fields[3].to_string(),
            idle_seconds,
            busy: idle_seconds < BUSY_WINDOW_SECONDS,
        });
    }
    sessions
}

/// The agent whose binary appears in a pane's command line, if any.
///
/// The agents are tried in display order, so a pane that names two of them reads
/// as the first one. The token's last path segment is compared, because the
/// launcher starts the real binary by its absolute path.
fn agent_in(command: &str) -> Option<AgentKind> {
    let tokens: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|token| !token.is_empty())
        .collect();
    for agent in AgentKind::ALL {
        let expected = agent.as_str();
        if tokens.iter().any(|token| matches_binary(token, expected)) {
            return Some(agent);
        }
    }
    None
}

/// Whether one command-line token names the given binary.
///
/// `claude.exe` is accepted as well, because an agent may be a wrapped build.
fn matches_binary(token: &str, expected: &str) -> bool {
    let last = match token.rsplit_once('/') {
        Some((_, last)) => last,
        None => token,
    };
    last == expected || last.strip_suffix(".exe") == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_emits_seven_tab_separated_fields() {
        assert!(
            PANES.contains(r"printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n'"),
            "the pane listing must emit tab-separated fields"
        );
        // tmux 3.4 rewrites a literal tab in `-F` to `_`, so no control
        // character may reach the format.
        assert!(!PANES.contains('\t'), "the tmux format must not hold a literal tab");
        assert!(PANES.contains("IFS='|'"));
        assert!(PANES.contains("tmux list-panes -a"));
        assert!(PANES.contains("/proc"));
    }

    #[test]
    fn a_claude_pane_that_just_printed_is_busy() {
        let output = "\
nestbox-t-a\t%1\t101\t/home/nestbox/work\t1000\t1005\tsh -c /home/nestbox/.local/bin/claude --continue
";
        let sessions = parse_panes(output);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentKind::Claude);
        assert_eq!(sessions[0].tmux_session, "nestbox-t-a");
        assert_eq!(sessions[0].pane_id, "%1");
        assert_eq!(sessions[0].pid, 101);
        assert_eq!(sessions[0].path, "/home/nestbox/work");
        assert_eq!(sessions[0].idle_seconds, 5);
        assert!(sessions[0].busy);
    }

    #[test]
    fn a_shell_pane_is_not_a_session_and_a_quiet_codex_pane_is_not_busy() {
        let output = "\
nestbox-t-b\t%2\t202\t/tmp\t900\t1000\tbash -l
nestbox-t-c\t%3\t303\t/home/nestbox/app\t900\t1000\t/bin/bash | /usr/local/bin/codex resume --last
";
        let sessions = parse_panes(output);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent, AgentKind::Codex);
        assert_eq!(sessions[0].idle_seconds, 100);
        assert!(!sessions[0].busy);
    }

    #[test]
    fn a_short_line_is_skipped() {
        let output = "nestbox-t-d\t%4\t404\tsh -c opencode\n";
        assert!(parse_panes(output).is_empty());
    }

    #[test]
    fn a_wrapped_binary_still_counts_and_the_earlier_agent_wins() {
        assert_eq!(agent_in("/opt/agent/claude.exe --continue"), Some(AgentKind::Claude));
        assert_eq!(
            agent_in("/usr/bin/opencode run | /usr/bin/codex"),
            Some(AgentKind::Codex)
        );
    }

    #[test]
    fn only_the_shapes_the_supervisor_creates_are_accepted() {
        assert!(valid_agent_name("openchamber"));
        assert!(valid_agent_name("recipe_agent-2"));
        assert!(!valid_agent_name(""));
        assert!(!valid_agent_name("../escape"));
        assert!(!valid_agent_name("two words"));
        assert!(!valid_agent_name(&"a".repeat(65)));
    }
}
