# ADR 0055 — An agent's sign-in mode lives in a launcher, not the container

Status: Accepted
Date: 2026-11-01

## Context

Whether an agent used its own subscription or the gateway was a create-time choice:
the gateway key and its base URLs arrived as container environment variables, and the
container environment cannot be changed while the container runs (ADR 0053).
Sign-in state lived in `.agent-state` for Claude Code only, so Codex and OpenCode lost
their login on every recreate. The result: switching the account cost a new sandbox
and the work inside it.

The key itself must not move (ADR 0016): the control plane resolves it at use, and the
container is the place it is used from.

## Decision

**Every switchable agent is started through a launcher, and the launcher reads the
mode from a file.**

* `$HOME/.nestbox/bin/claude`, `.../codex`, `.../opencode` hold the same text and
  dispatch on `basename "$0"`. The launcher is written at create, at recreate and at
  every mode change, so it always matches the build that is running.
* The mode is one line in `$HOME/.nestbox/mode/<agent>`: `subscription` or `gateway`.
* `gateway` → `exec "$real" "$@"`. `subscription` → the same, with the gateway
  variables removed from the child environment (`ANTHROPIC_AUTH_TOKEN`,
  `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL`,
  `ANTHROPIC_SMALL_FAST_MODEL` for Claude; `OPENAI_API_KEY`, `OPENAI_BASE_URL`,
  `OPENAI_API_BASE`, `OPENAI_MODEL` for OpenCode). The real binary is the first
  `$d/<agent>` on `PATH` that is executable and is not the launcher's own directory,
  so the launcher can be first on `PATH` without recursion.
* **Config files follow the mode too.** `$HOME/.nestbox/env.sh` exports `PATH`, the
  proxy and the trust bundle and is sourced from one block in `.bashrc` and `.profile`
  between `# >>> nestbox >>>` and `# <<< nestbox <<<`. The per-agent config
  directories (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `XDG_DATA_HOME`) point into
  `$HOME/.agent-state/` when that directory exists, so **every login-agent pack gets a
  persistent `.agent-state` home** and a sign-in survives a recreate.
* **The sign-in rules follow the agent.** The login hosts per agent are created and
  removed as named network rules (`Sign-in (<agent>): <host>`) when the mode changes.
* **Switching restarts agent sessions, and nothing else.** Idle sessions restart
  immediately, busy ones when they go idle (`restart=idle`), or all of them now, or
  nothing now and a pending mark (`later`). A restart is a tmux respawn of the pane
  with the agent's resume arguments (`--continue`, `resume --last`, `--continue`), so
  the conversation continues in the new mode. Supervised agents are restarted through
  `$HOME/.nestbox/bin/nestbox-agentctl`.
* **The login itself is never touched.** The control plane does not read, copy or
  write an agent's token or account files.

## Alternatives rejected

* **Injecting the key at the proxy for every request.** That is the end state ADR 0051
  works towards, but it needs the accounting proxy to cover every model dialect and it
  does not fix the config files. The launcher works today and needs no image rebuild.
* **A separate container per agent and mode.** Two sandboxes to keep in sync, twice the
  memory, and the work still lives in one of them.
* **Rewriting the environment of running processes through `docker exec`.** It is not
  possible in general, and it would leave running agents disagreeing with each other.
* **Only supporting Claude.** Codex and OpenCode are login agents in the packs and lost
  their state on recreate; leaving them out would keep the bug for two of three agents.

## Consequences

An agent started by absolute path bypasses the launcher and keeps the gateway
environment — the launcher is the contract, and the packs put it first on `PATH`. A new
provider variable has to be added to the launcher's strip list, which is a one-line
change in one file. A `subscription` sandbox with no login shows the provider's
sign-in prompt inside the agent, which is the intended flow; the console reports the
account it can see, or none.

## Invalidation conditions

When the accounting proxy covers every model dialect the agents use **and** the agent
configuration no longer needs per-mode files, the launcher collapses into a proxy
decision and this ADR is superseded.
