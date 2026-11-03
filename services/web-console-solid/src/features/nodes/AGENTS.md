# nodes/ — the machines sandboxes and models run on

## Purpose

Answer "where does this run" and "why was that model refused" for one machine at
a time. A node's capacity is not a stored field: it is the last hardware
measurement minus what the models installed on that node reserve. This screen is
the only place those two facts are shown together.

## Rules

- **No invented capacity.** `hardware_known: false` means nobody has probed the
  node and the budget is zeros. The screen says "not measured yet" and draws no
  bars; a zero-length bar would look like a measured zero.
- The numbers are the server's (`budget.ram_*`, `budget.vram_*`, `cpu_cores`,
  `gpu_count`, `installs`, `sandboxes`). The client only formats them.
- The budget is a snapshot: the last measurement minus reservations. It is not a
  live reading of free memory, and the page says so in words rather than
  implying a gauge.
- `POST /nodes/{id}/probe` is the only way a budget appears. On
  `503 node.runtime_unavailable` the drawer shows the server's message in a
  warning, not an error: the node is still registered, the runner just did not
  answer.
- Only the display fields are writable (`display_name`, `description`). `name` is
  the identifier in URLs; kind, status, and `container_gpu` are measured. The
  drawer says that next to the form instead of offering the fields.
- A sandbox list link needs a route the feature does not own: `props.onOpenNode`
  (from the inventory) and `props.onOpenSandbox` (from this drawer) are optional
  callbacks. Without them the names render as text, never as a button that does
  nothing.
- **The tunnel column is the hub's opinion, not a ping.** It comes from
  `nodes/tunnels` (every ten seconds, and at once on a `node_tunnel_changed`
  event); a node the hub has no peer for reads "no tunnel", never "up". The local
  host reads "local" in both the tunnel and the egress column, because it is this
  gateway and takes no tunnel to itself.
- **The egress select never shows an unconfirmed choice.** A refused write leaves
  the control at the value the page knew and the error above the table; `strict`
  asks first, because it moves all of the node's traffic through this gateway.
  The node list does not report the current lock yet, so the select starts at
  "not read" rather than assuming `strict`.
- **A credential never stays on the page.** `ConnectNode` empties its password and
  key fields as soon as the request is answered, and the server is not allowed to
  store them at all.
- **The table has one column per fact, and every column is visible (V25).** It is
  `table-layout: fixed` with the widths in `styles/nodes.css`; the NODE column takes
  what is left and never wraps — the name, the key and the description are one line
  with an ellipsis and a `title`. A long budget, a wide badge or a wrapped name must
  not push ACTIONS off the right edge, which is what happened when the budget cell
  grew its own column.
- **The budget cell is its own column, not a sidebar.** In the table it is the
  compact `BudgetBars`: each bar keeps its label and track on one line with the
  numbers under it, so RAM, GPU memory and the install sentence stay inside 260 px.
- **A row can be probed without opening it.** PROBE in ACTIONS calls
  `POST /nodes/{id}/probe` and reloads the list; the button is disabled while the
  runner answers, so a slow node is not asked twice.

## Files

| File | Responsibility |
|---|---|
| `NodesPage.jsx` | The table: name and key, kind, status, tunnel state, egress lock, container GPU, sandbox count, budget; fixed column widths (V25), a PROBE action per row; opens the drawer; keeps the tunnel list fresh |
| `NodeDrawer.jsx` | One node: facts, budget, probe, its sandboxes, display rename |
| `NodeWizard.jsx` | Add node wizard: source (script, connect by IP, rent), placement, details, optional model, review; Finish makes one `POST /nodes/quick` and follows the job |
| `nodeWizard.js` | Pure wizard logic: `steps`, `validate`, `bodyFor`, `reviewLines`, `progressStates`; no JSX |
| `nodeWizard.test.js` | Its rules, run with `node --test` |
| `JobProgress.jsx` | A job's step ticks, last log line and optional join-command copy box; polls every 3 s plus live `job` events |
| `DeleteNodeDialog.jsx` | Delete a node: loads the summary, drains first (move or stop only) when it is not empty |
| `JoinNode.jsx` | Add a machine by handing out a one-time join command |
| `ConnectNode.jsx` | Add an existing machine by address: SSH login used once, WireGuard port, egress lock |
| `TunnelBadge.jsx` | One node's tunnel as a badge: state word, handshake age, colour |
| `tunnels.js` | Tunnel wording and colour: `tunnelLabel`, `ageText`, `tunnelTone`; no JSX |
| `tunnels.test.js` | The wording rules, run with `node --test` |
| `BudgetBars.jsx` | Total / reserved / free for RAM and GPU memory, plus cores, GPUs, and installs |
| `useNodes.js` | The list and the drawer's state: fetch, probe, rename; no JSX |
| `nodeFormat.js` | Status colour and kind wording shared by the page and the drawer |

## Left to the integrator

- A route and a nav entry for `NodesPage` (default export, named `NodesPage`).
- `onOpenNode={(id) => ...}` on `SandboxListPage` and `onOpenSandbox={(id) => ...}`
  on `NodesPage`, if the console should link between the two screens.

## Not here

Grouping the cluster topology by node. The `/topology` answer carries no node
identity per sandbox, so the only client-side way is one sandbox request per node
plus a second placement model in the browser — that duplicates the server's
placement and would restructure the graph. It belongs in an endpoint change.

`JoinNode.jsx` and `ConnectNode.jsx` are no longer mounted by `NodesPage` (the wizard replaced them) but are kept.
