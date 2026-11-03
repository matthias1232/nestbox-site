# traffic/ — the live console

## Purpose

The promise: see what **every** sandbox is doing **now**, and see the minutes
before it. One table, one window, one stream — plus the two things an operator
does with what they see: open a firewall for a while, and decide about a block.

## The two views

| View | Question | Source |
|---|---|---|
| **Live traffic** | What is happening, and where? | `GET /traffic` (window) + the SSE stream |
| **Decisions** | What must I decide? | `GET /traffic/decisions`, denied destinations grouped |

The split is deliberate. The live view shows everything, including what passed;
the decisions view shows only what was stopped, grouped by what a person can
allow, because "what happened" and "what must I decide" are different questions.

## How the live view works

- Opening the page reads the environment-wide window first and then subscribes to
  the app's shared telemetry stream (`state/live.jsx`; the view holds one
  `wantTelemetry()` while mounted, and the pop-out uses the same connection
  budget as any other window). The stream only carries what arrives after it
  opened, so without the read an idle sandbox looks like a broken page and a
  reload loses the minutes before it.
- Both sources are normalised to one row shape. The stream carries less than the
  stored row: a live event knows the destination and the decision, a stored row
  also knows the method, path and MCP tool. The table shows the difference rather
  than an empty cell that looks like a bug.
- Rows are de-duplicated by event id: a re-read and a repeated frame are the same
  observation.
- The buffer is capped. An unbounded buffer is a memory leak that appears an hour
  into an incident, which is exactly when an operator is watching.
- Pause stops rendering, not the connection, so resuming shows what happened.

## Filters

- **Decision** (`blocked`/`allowed`) and **sandbox** are server filters: changing
  one re-reads the window, so the counts and the rows describe the same thing.
- **Kind** and the **search box** are view filters over the loaded rows.
- `dns` rows are their own event kind on the server. A DNS row is badged with the
  `dns` channel; it never opened a connection.

## Master switches

A switch is not a setting. It is a **traffic window**: time-bounded, recorded, and
scoped to one firewall (proxy, DNS, MCP, gateway). Two modes:

- **bypass** — allow, and say nothing more;
- **audit** — *observation mode*: allow, and record the decision as a block, so a
  policy can be built against real traffic without breaking the workload.

The panel says so in a banner while observation is on. The OpenShell supervisor is
listed and never switchable: it carries the sandbox's API and MCP forwarding, so
switching it off would cut the sandbox off from its tools rather than open traffic.

The panel sits in a 280–480 px side column, so each channel is a **card**, never a
table row: the channel name (bold) and its one-word state (`enforcing`, `open`,
`audit`) share the header line, the description follows and is clamped to three
lines, and the duration buttons wrap underneath. While a window is open its
countdown (`open · 12 min`, `observing · 12 min`) and **Revoke** join that action
row. A four-column table in the same width squeezes the description to one word
per line, which is what F17 showed; keep the card layout when the panel gets a
width of its own.

## Decisions

Three answers, and only three:

- **for a time** — `POST /traffic-windows` on the destination's channel;
- **permanently** — a rule draft, reviewed and approved (ADR 0011);
- **not at all** — hide the row. Hiding is session-local and never changes a
  policy; hidden targets keep being denied and are reachable through "show
  hidden".

## Rules

- A rule is never created from a click. A click creates a **draft**.
- A rendering error must not close the stream. It cannot: the connection belongs
  to `state/live.jsx`, and this page only unsubscribes when it unmounts.
- Every row names its sandbox. The question is "what is happening, and where"; an
  id alone does not answer it.

## Files

| File | Responsibility |
|---|---|
| `useTrafficStream.js` | History read, the app's shared telemetry stream (subscribed with `live.on('network'/'dns'/'inference')`, one `wantTelemetry()` held while the view is mounted), normalisation, buffer, pause, filters |
| `TrafficPage.jsx` | `TrafficView` (shared with the pop-out) and the page: header, three filters, red lane, table, side panel, draft card |
| `trafficFilters.js` | Pure kind / decision / target filters, chip counts and target options (tested in `trafficFilters.test.js`) |
| `TrafficPopout.jsx` | The `/pop/traffic` window: the same view without the shell |
| `useWindows.js` | The master switches: read the windows in force, open one, revoke one |
| `MasterSwitches.jsx` | The switch panel (markup only): one card per channel, name and state, description, durations |
| `useDecisions.js` | Denied targets, hide/show, allow-for-a-time, draft |
| `DecisionsPanel.jsx` | The decisions table (markup only) |

## Control Center layout (F17)

One table with three always-visible filters: Kind (all/proxy/DNS/MCP/inference), Decision
(all/allowed/refused/would deny) and Sandbox or node. Refused destinations also sit in a red
lane above the table with inline answers (allow now, 15 min, 1 h, draft rule, ignore). The
master switches and the decisions list moved into a collapsible side panel. Styles: `styles/traffic.css`.
