//! The launcher inside a sandbox that starts each agent in its chosen sign-in
//! mode (ADR 0054).
//!
//! `install` writes `$HOME/.nestbox/bin/<agent>` (one script, dispatching on its
//! own name), `nestbox-agentctl`, `.nestbox/env.sh` (PATH, proxy and CA trust) and
//! the hook in `.bashrc` / `.profile`. `write_mode` flips one agent's mode file.
//! Nothing restarts, nothing rebuilds: the next process of that agent reads it.
//!
//! A mode change is therefore cheap and reversible: the gateway credential never
//! leaves the container environment, the launcher only decides whether the agent
//! process inherits it, and a user who signs in with their own account still has
//! the same files on the same persistent home.

use std::collections::BTreeMap;

use base64::Engine;

use super::ServiceContext;
use crate::domain::agent_auth::{AgentAuthMode, AgentKind};
use crate::error::{ApiError, ErrorCode};

/// The script behind `$HOME/.nestbox/bin/claude`, `.../codex` and `.../opencode`.
///
/// One text installed under three names, because the three agents differ only in
/// where their configuration lives and in which environment keys carry the
/// gateway credential. It finds the real binary by walking `PATH` and skipping its
/// own directory, so `claude` typed in a shell still reaches the vendor binary.
const LAUNCHER: &str = r#"#!/bin/sh
# Nestbox launcher: starts the real agent with the sign-in mode chosen in the console.
# Written by Nestbox on every change; edit the console, not this file.
[ -f "$HOME/.nestbox/env.sh" ] && . "$HOME/.nestbox/env.sh"
agent="$(basename "$0")"
mode="$(cat "$HOME/.nestbox/mode/$agent" 2>/dev/null || echo gateway)"
self_dir="$HOME/.nestbox/bin"
real=""
old_ifs="$IFS"; IFS=:
for dir in $PATH; do
  [ "$dir" = "$self_dir" ] && continue
  if [ -x "$dir/$agent" ]; then real="$dir/$agent"; break; fi
done
IFS="$old_ifs"
if [ -z "$real" ]; then echo "nestbox: $agent is not installed in this sandbox" >&2; exit 127; fi
state="$HOME/.agent-state"
case "$agent" in
  claude) [ -d "$state" ] && export CLAUDE_CONFIG_DIR="$state/claude" ;;
  codex) if [ -d "$state" ]; then export CODEX_HOME="$state/codex"; mkdir -p "$CODEX_HOME"; fi ;;
  opencode)
    if [ -d "$state" ]; then
      export XDG_DATA_HOME="$state/xdg-data"
      if [ ! -d "$XDG_DATA_HOME/opencode" ] && [ -d "$HOME/.local/share/opencode" ]; then
        mkdir -p "$XDG_DATA_HOME" && cp -a "$HOME/.local/share/opencode" "$XDG_DATA_HOME/opencode"
      fi
    fi ;;
esac
if [ "$mode" = subscription ]; then
  exec env -u ANTHROPIC_AUTH_TOKEN -u ANTHROPIC_API_KEY -u ANTHROPIC_BASE_URL -u ANTHROPIC_MODEL \
    -u ANTHROPIC_SMALL_FAST_MODEL -u OPENAI_API_KEY -u OPENAI_BASE_URL -u OPENAI_API_BASE \
    -u OPENAI_MODEL "$real" "$@"
fi
exec "$real" "$@"
"#;

/// `$HOME/.nestbox/bin/nestbox-agentctl`: look at the supervised agents, restart one.
///
/// The status comes from the PID files the supervisor loop writes, not from a
/// process scan, so it names the agents Nestbox knows about and nothing else.
const AGENTCTL: &str = r#"#!/bin/sh
# Nestbox agent control: status of supervised agents, and restart one of them.
dir=/tmp/nestbox-agents
case "${1:-}" in
  status)
    for f in "$dir"/*.pid; do
      [ -e "$f" ] || continue
      name="$(basename "$f" .pid)"; pid="$(cat "$f")"
      if kill -0 "$pid" 2>/dev/null; then echo "$name running $pid"; else echo "$name stopped"; fi
    done ;;
  restart)
    [ -n "${2:-}" ] || { echo "usage: nestbox-agentctl restart <name>" >&2; exit 2; }
    f="$dir/$2.pid"
    [ -s "$f" ] || { echo "nestbox: no supervised agent named $2" >&2; exit 1; }
    kill -TERM "$(cat "$f")" ;;
  *) echo "usage: nestbox-agentctl status | restart <name>" >&2; exit 2 ;;
esac
"#;

/// The install script: writes the three files, the mode files, the CA bundle and
/// the shell hook. Every write goes through a `.new` file and a `mv`, so a reader
/// never sees half a script.
const INSTALL: &str = r#"set -eu
d="$HOME/.nestbox"
mkdir -p "$d/bin" "$d/mode"
printf %s "$1" | base64 -d > "$d/bin/.launcher.new"
for a in claude codex opencode; do
  cp "$d/bin/.launcher.new" "$d/bin/$a.new"; chmod 755 "$d/bin/$a.new"; mv "$d/bin/$a.new" "$d/bin/$a"
done
rm -f "$d/bin/.launcher.new"
printf %s "$2" | base64 -d > "$d/bin/nestbox-agentctl.new"
chmod 755 "$d/bin/nestbox-agentctl.new"; mv "$d/bin/nestbox-agentctl.new" "$d/bin/nestbox-agentctl"
printf %s "$3" | base64 -d > "$d/env.sh.new"; mv "$d/env.sh.new" "$d/env.sh"
shift 3
for pair in "$@"; do
  printf '%s\n' "${pair#*=}" > "$d/mode/${pair%%=*}"
done
if [ -s /run/secrets/egress-ca.crt ]; then
  src="${SSL_CERT_FILE:-/etc/ssl/certs/ca-certificates.crt}"
  [ "$src" = "$d/ca-bundle.crt" ] && src=/etc/ssl/certs/ca-certificates.crt
  [ -s "$src" ] || src=/etc/ssl/certs/ca-certificates.crt
  if cat "$src" /run/secrets/egress-ca.crt > "$d/ca-bundle.crt.new" 2>/dev/null; then
    mv "$d/ca-bundle.crt.new" "$d/ca-bundle.crt"
  fi
fi
for rc in "$HOME/.bashrc" "$HOME/.profile"; do
  [ -f "$rc" ] || : > "$rc"
  if ! grep -q '# >>> nestbox >>>' "$rc"; then
    printf '\n# >>> nestbox >>>\n[ -f "$HOME/.nestbox/env.sh" ] && . "$HOME/.nestbox/env.sh"\n# <<< nestbox <<<\n' >> "$rc"
  fi
done
"#;

/// The one-line script that flips a single agent's mode. Small on purpose: it runs
/// on every switch and must not depend on anything `install` left behind.
const WRITE_MODE: &str = r#"set -eu
mkdir -p "$HOME/.nestbox/mode"
printf '%s\n' "$2" > "$HOME/.nestbox/mode/$1"
"#;

/// Quote a value for a POSIX shell: wrap in single quotes, escape single quotes.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The text of `.nestbox/env.sh` for the given proxy environment.
///
/// The launcher and every login shell source this, so the sandbox's own tools and
/// the agents agree about where the proxy is and which CA bundle to trust. The
/// proxy values are quoted: they are operator input, not shell code.
fn render_env_sh(egress_env: &BTreeMap<String, String>) -> String {
    let mut out = String::from(
        "# Written by Nestbox on every change; edit the console, not this file.\n\
         case \":$PATH:\" in *\":$HOME/.nestbox/bin:\"*) ;; *) export PATH=\"$HOME/.nestbox/bin:$PATH\" ;; esac\n\
         if [ -s \"$HOME/.nestbox/ca-bundle.crt\" ]; then\n\
         \x20 for v in SSL_CERT_FILE REQUESTS_CA_BUNDLE CURL_CA_BUNDLE GIT_SSL_CAINFO AWS_CA_BUNDLE; do export \"$v=$HOME/.nestbox/ca-bundle.crt\"; done\n\
         \x20 [ -s /run/secrets/egress-ca.crt ] && export NODE_EXTRA_CA_CERTS=/run/secrets/egress-ca.crt\n\
         fi\n",
    );
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        if let Some(value) = egress_env.get(key) {
            out.push_str(&format!("export {key}={}\n", shell_quote(value)));
        }
    }
    out
}

/// Install or refresh the launcher, agentctl and environment in a sandbox.
///
/// Runs inside the container because only the container knows where the vendor
/// binaries are and which CA bundle it has; the control plane only supplies the
/// text. A sandbox that cannot be reached is a 503 rather than a silent success,
/// so a mode change never claims to have taken effect when it did not.
pub async fn install(
    ctx: &ServiceContext,
    sandbox_name: &str,
    modes: &[(AgentKind, AgentAuthMode)],
    egress_env: &BTreeMap<String, String>,
) -> Result<(), ApiError> {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut command = vec![
        "sh".to_string(),
        "-c".to_string(),
        INSTALL.to_string(),
        "sh".to_string(),
        engine.encode(LAUNCHER),
        engine.encode(AGENTCTL),
        engine.encode(render_env_sh(egress_env)),
    ];
    for (agent, mode) in modes {
        command.push(format!("{}={}", agent.as_str(), mode.as_str()));
    }
    match ctx.runtime.exec(sandbox_name, &command, 30).await {
        Ok(result) if result.exit_code == Some(0) => Ok(()),
        Ok(result) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!(
                "The agent launcher could not be installed (exit {:?}): {}",
                result.exit_code,
                result.output.chars().take(300).collect::<String>()
            ),
        )),
        Err(error) => Err(ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, error.to_string())),
    }
}

/// Write one agent's mode file.
///
/// Deliberately does not reinstall the launcher: the launcher reads the mode file
/// on every start, so a switch is one small write and cannot fail on an unrelated
/// file. A running process is restarted by `agent_auth_service`, not here.
pub async fn write_mode(
    ctx: &ServiceContext,
    sandbox_name: &str,
    agent: AgentKind,
    mode: AgentAuthMode,
) -> Result<(), ApiError> {
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        WRITE_MODE.to_string(),
        "sh".to_string(),
        agent.as_str().to_string(),
        mode.as_str().to_string(),
    ];
    match ctx.runtime.exec(sandbox_name, &command, 15).await {
        Ok(result) if result.exit_code == Some(0) => Ok(()),
        Ok(result) => Err(ApiError::unavailable(
            ErrorCode::NODE_RUNTIME_UNAVAILABLE,
            format!(
                "The sign-in mode of {} could not be written (exit {:?}): {}",
                agent.as_str(),
                result.exit_code,
                result.output.chars().take(300).collect::<String>()
            ),
        )),
        Err(error) => Err(ApiError::unavailable(ErrorCode::NODE_RUNTIME_UNAVAILABLE, error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_env() -> BTreeMap<String, String> {
        let mut environment = BTreeMap::new();
        environment.insert("HTTPS_PROXY".into(), "http://egress-proxy:3128".into());
        environment.insert("NO_PROXY".into(), "localhost,egress-proxy".into());
        environment
    }

    /// Write a script to a throw-away file and ask the shell whether it parses.
    fn parses(script: &str, name: &str) -> bool {
        let path = std::env::temp_dir().join(format!("nestbox-b01-{}-{name}", std::process::id()));
        if std::fs::write(&path, script).is_err() {
            return false;
        }
        let status = std::process::Command::new("sh").arg("-n").arg(&path).status();
        let _ = std::fs::remove_file(&path);
        status.map(|status| status.success()).unwrap_or(false)
    }

    #[test]
    fn a_value_with_a_quote_survives_the_shell() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn the_environment_names_the_proxy_in_both_spellings() {
        let mut environment = proxy_env();
        environment.insert("http_proxy".into(), "http://egress-proxy:3128".into());
        let text = render_env_sh(&environment);
        assert!(text.contains("export HTTPS_PROXY='http://egress-proxy:3128'"));
        assert!(text.contains("export http_proxy='http://egress-proxy:3128'"));
        assert!(!text.contains("export ALL_PROXY="), "only what the sandbox was given");
    }

    #[test]
    fn the_environment_puts_the_launcher_first_on_the_path() {
        let text = render_env_sh(&proxy_env());
        assert!(text.contains(".nestbox/bin"));
        assert!(text.contains("ca-bundle.crt"));
    }

    #[test]
    fn the_launcher_strips_the_gateway_credential_in_subscription_mode() {
        assert!(LAUNCHER.contains("env -u ANTHROPIC_AUTH_TOKEN"));
        assert!(LAUNCHER.contains("exec \"$real\" \"$@\""));
        assert!(LAUNCHER.contains("$HOME/.agent-state"));
    }

    #[test]
    fn every_script_the_service_writes_parses_in_a_posix_shell() {
        assert!(parses(LAUNCHER, "launcher"));
        assert!(parses(AGENTCTL, "agentctl"));
        assert!(parses(INSTALL, "install"));
        assert!(parses(WRITE_MODE, "write-mode"));
        assert!(parses(&render_env_sh(&proxy_env()), "env.sh"));
    }

    #[test]
    fn the_agentctl_reports_and_restarts_by_pid_file() {
        assert!(AGENTCTL.contains("/tmp/nestbox-agents"));
        assert!(AGENTCTL.contains("kill -TERM"));
    }
}
