# ADR 0041 — A name lookup is its own event, and a master switch is a bounded window

Status: Accepted
Date: 2026-10-30

## Context

The Live traffic page looked dead in the one case an operator needs it: a quiet
environment. Three separate reasons, each verified against the running appliance:

1. **The page only listened.** It opened the stream and never read the window that
   had already happened, so a reload lost the last minutes and an idle sandbox was
   indistinguishable from a broken screen.
2. **A name lookup was stored as a connection.** The supervisor reports DNS
   decisions, and the ingest wrote them as `kind = network` with the protocol word
   `dns`. The DNS facet filters on the kind, so it was always empty, and the row had
   no host to show because a DNS event names a `hostname`, not a destination.
3. **The tab list lived in the browser.** `localStorage` remembered tab ids, but
   nothing told the sandbox which shells were alive. After a reload an application
   could be replaced by a fresh shell, and an image without `tmux` gave no warning
   at all.

The operator also asked for two things the model had no shape for: "allow
everything for a few minutes, per firewall", and "observe what would be blocked
without breaking the workload".

## Decision

- **A name lookup is its own event kind (`dns`) with its own channel.** The ingest
  writes a `DnsEvent` for it, and the traffic read treats `network` and `dns` rows
  as the same evidence, because "what did this sandbox reach" means both. The
  destination expression accepts the third spelling (`hostname`).
- **The console reads the window, then listens.** `GET /api/v1/traffic` answers for
  the whole environment with the sandbox's name on every row; the page reads it on
  open and appends the stream afterwards, de-duplicated by event id. A live row
  admits it is coarser than the stored row rather than showing an empty cell.
- **A window carries a channel and a mode.** `channel` is the firewall (`proxy`,
  `dns`, `mcp`, `gateway`); `mode` is `bypass` (allow quietly) or `audit`
  (**observation**: allow and record the decision as a block). Both default to the
  previous behaviour, so an existing window is a bypass for every channel.
- **The OpenShell supervisor is listed and never switchable.** It carries the
  sandbox's API and MCP forwarding; switching it off would cut the sandbox off from
  its tools rather than open traffic. The panel says so instead of offering a
  control that would break the sandbox.
- **A denial has exactly three answers**: open it for 15/30/60 minutes on its
  channel, draft a permanent rule (reviewed before it applies, ADR 0011), or hide
  the row. Hiding is a view choice, never a policy change.
- **The terminal tab list is a table.** `sandbox_terminal_tabs` stores the tabs one
  user keeps in one sandbox, the API reports which tmux sessions still exist, and
  the console warns when the image has no `tmux` instead of quietly starting a shell
  that cannot survive.

## Consequences

- A DNS deny now appears in the DNS facet and in the traffic list as a name lookup.
  Rows written before this change keep their old shape; they are read through the
  same destination expression, so they do not disappear.
- Observation mode is visible: the console shows a banner while it is on, and the
  traffic rows carry `audit`. A bypass has no banner by design — it is a deliberate,
  time-bounded action with a written reason, and the window list shows it.
- The master switches are per environment. A sandbox-scoped window remains the
  narrower tool and is what the decisions view opens.
- **RTP is not captured.** The proxy terminates TCP; RTP would need a UDP tap on the
  node. The vocabulary has no `rtp` kind because there is no producer, and inventing
  one would make the console claim a coverage the runtime does not have.
- A tab whose shell ended stays listed with `alive: false`. It is a place to
  reconnect to, not a tab to hide.
