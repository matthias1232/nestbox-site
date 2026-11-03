# ADR 0056 — A sandbox session is shared by default, and watching is visible

Status: Accepted
Date: 2026-11-01

## Context

Terminals belonged to one browser: the tab list lived in `localStorage`, the tmux
session was named after that browser's tab, and a colleague — or the admin — had no way
into a shell that was already running an agent. SSH opened its own `bash -lc` and never
landed in the tmux session, so disconnecting ended the work. "Who is connected" was a
number on a card, not a list. The desktop had no read-only mode at all: whoever was
given the page could type.

A workbox is a shared operating object: the owner, a colleague and an admin look at the
same sandbox, and the tool sessions are part of the sandbox, not part of a browser.

## Decision

* **Terminal tabs belong to the sandbox.** A tab is the tmux session `nestbox-<tab>` in
  the sandbox; anyone with `SandboxExec` on the sandbox may open any tab. The tab list
  is stored per sandbox, not per browser, and a tab survives a reload and a reconnect.
* **Presence is a list, not a count.** A terminal, an SSH session and a desktop viewer
  each write a row (`kind` = `terminal` | `ssh` | `desktop`, with `tab_id` and
  `view_only`), closed when they disconnect; `GET /api/v1/sandboxes/{id}/presence`
  reports the live ones, and the console shows who is connected and what they are in.
* **SSH lands in the shared session.** No command → the tmux session `nestbox-ssh`; a
  command `attach <tab>` or `tab <tab>` → `nestbox-<tab>`; any other command →
  `bash -lc`. A disconnect detaches and does not end the session; there is no Ctrl-D
  way to destroy a shared shell by accident.
* **View-only is enforced on the server.** A principal without `SandboxExec` but with
  `SandboxRead` is always view-only: the runner drops keyboard, pointer, clipboard and
  resize messages from the desktop websocket, instead of hiding buttons in a browser
  that could be scripted.
* **Watching is visible to the owner.** Everyone who is connected appears in the
  presence list of the sandbox, so a joined session is never silent.
* **Limits are counted server side**: 4 open terminals per user per sandbox, 12 per
  sandbox, 20 per user across sandboxes. Closing a tab kills `nestbox-<tab>` and
  removes it for every user.

## Alternatives rejected

* **A consent dialog before an admin joins.** It turns a shared operating object into a
  negotiation and breaks the case that matters — an operator helping while something
  is broken. A visible presence list is the honest answer, and it is not optional.
* **Per-user tmux servers.** Isolation without a requirement, and it multiplies the
  processes an agent runs in.
* **A new websocket fan-out layer.** The existing terminal socket and the SSE stream
  already carry what is needed; a second transport would be a second source of truth
  about who is connected.
* **Hiding the controls of a view-only desktop in the console.** The browser is not the
  boundary; the runner is.

## Consequences

Two people can type into the same shell at the same time. That is intended and it is the
reason for the presence list, not a defect to fix with a lock. A tab whose tmux session
died is reported as ended rather than silently replaced, so nobody types into a fresh
shell believing it is the old one. A viewer sees the desktop without being able to act,
and the UI says so. See ADR 0055 for the agents that run inside these sessions and how a
mode change restarts them without ending the conversation.

## Invalidation conditions

When a tenant requires per-user isolation of tool sessions, a tab needs an owner and
joining needs a grant; the presence rows then carry the grant instead of only the
session.
