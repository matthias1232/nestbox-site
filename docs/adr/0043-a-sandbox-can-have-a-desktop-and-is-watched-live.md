# 0043 — A sandbox can have a desktop, and is watched live

## Context

Agents need a real browser (log in, click, take screenshots), and a person has
to be able to see and take over what the sandbox shows. Operators also want to
see, per sandbox and live, which processes talk to whom, what the models are
asked, and to release a blocked destination without a detour through drafts.

## Decision

* **Desktop as a pack.** `desktop` is an install kind with fixed scripts shipped
  in `tool-images/desktop`. XFCE, not GNOME: GNOME needs systemd and a session
  bus the container does not have and costs about twice the memory.
* **No port is published.** VNC listens on `127.0.0.1:5900` in the sandbox. The
  runner reaches it with `docker exec` and a constant Python bridge; the control
  API proxies the WebSocket after the origin, permission (`SandboxExec`) and
  state checks. Pixels are never recorded.
* **One Chrome for person and agent.** Chrome runs on the desktop with remote
  debugging on loopback; `chrome-devtools-mcp` (stdio, registry key
  `chrome-browser`) attaches to it. A login done by hand on the desktop is the
  login the agent uses. The "Claude in Chrome" extension was rejected: it needs
  its own claude.ai login per browser and cannot be configured automatically.
* **Chrome trusts the egress CA only when the runner mounted one**, through its
  NSS database at start, never at build time (the CA is not in any image).
* **Live views are samples, honestly labelled.** Connections are read from
  `/proc` by a fixed probe as the sandbox user (root has no more reach once
  capabilities are dropped). A socket that opens and closes between two polls is
  not seen; lossless tracing would need host-level eBPF and privileges we do not
  grant.
* **Chats are read from recorded bodies**, so they need an inspecting egress
  mode. A long request keeps its first 8 KiB and its last 56 KiB: the model and
  system prompt are at the start, the newest message at the end.
* **Release is one step.** `POST /sandboxes/{id}/allow` creates an enabled,
  sandbox-scoped allow rule (idempotent) that appears in the sandbox's rule list
  and can be deleted. Drafts remain for review workflows.

## Consequences

Desktop sandboxes need about 2.5–3 GiB. Chat contents are sensitive: they are
visible to the owner and to admins like every other sandbox view and are kept for
the existing 24 hours.
