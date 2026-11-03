# sandboxes/ — the inventory and the detail drawer

## Purpose

The screen an operator lives in: every sandbox, its live state, and the actions
on it.

## What the page must show

- The lifecycle status the control plane believes, and the runtime facts it
  observed, side by side. They can disagree, and that disagreement is the
  interesting information.
- Address, uptime, and resource limits, because "is it running" is not the same
  as "is it reachable".
- The node each sandbox runs on. The list already carries `node_name` and
  `node_id`; the column is the only place the console says which machine holds
  the container — and, at create time, what that machine has left to give.
- The log tail without leaving the page, because a failure is diagnosed from the
  log.

## Rules

- Paging is server-side. The page never asks for everything.
- A lifecycle action is disabled when the current status does not permit it, and
  the reason is the domain rule, not a guess in the component.
- A destructive action asks once, and says what will be destroyed.
- The node name links to the nodes screen only when `onOpenNode` is passed by the
  integrator; this feature does not own that route, and a dead link is worse than
  plain text.
- A resource pre-check compares the request against the budget `GET /nodes`
  returned. It never adds up capacity itself: a request above
  `budget.ram_free_bytes` or above `budget.cpu_cores` disables submit and names
  the number, and the server's own `422 node.insufficient_resources` /
  `404 node.not_found` is shown when it answers instead. A node nobody measured
  has no budget to check against, and the form says so rather than treating zero
  as full.
- Removing an agent decides what happens to its persistent home. The "keep its
  data" checkbox is that decision, it defaults to true, and `keep_persistent`
  carries it on the same DELETE.
- Sandboxes that belong together are one stack: a lead and its members, one level
  deep, drawn together in the list. Stacks are open by default, because a folded
  stack would hide a failing member; the collapsed lead shows the worst member
  status. The server decides what is a legal stack and its sentence is shown.

## Files

| File | Responsibility |
|---|---|
| `useSandboxes.js` | Paging, filters, selection, actions; no JSX |
| `SandboxListPage.jsx` | Table with stacks (a lead and its members drawn together), facets, bulk bar, the New sandbox drawer. Its Network and Connected columns come from `GET /sandboxes/control-overview` every 15 s; a Workbox button opens `/box/<id>` |
| `overview.js` | The list's live columns as text: `modeLabel`, `agentModes`, `indexById` for the overview map. Test: `node --test overview.test.js` |
| `ConnectPanel.jsx`, `AccessTab.jsx` | How to reach a sandbox: the node's address and hostname (never the container's), every published port with its dial address, the `ssh` line, an SSH config block, the one-time key. Each port says what it is, how to connect to it, and whether it answers (open, and something behind it). One component, used by the Access tab and the list's Connect drawer |
| `StackTopology.jsx`, `stackLayout.js` | The Topology tab: the stack from you to the internet drawn by `TopologyCanvas` (layers from the server), live every 5 s, with a detail panel of facts, connections and log below. `stackLayout.js` is pure (`node --test stackLayout.test.js`) |
| `MapTab.jsx` | Hands the sandbox to `StackTopology.jsx`; the tab is registered as Topology |
| `DashboardTab.jsx`, `ProcessesTab.jsx`, `InventoryTab.jsx`, `TrafficTab.jsx`, `McpTab.jsx`, `TerminalTab.jsx` | The deck's live tabs: usage and status, `docker top`, snapshot, every request with its decision (and "allow this"), MCP servers and calls, xterm.js over the terminal WebSocket. All poll every 4-5 s through `components/Poll.js` |
| `ResourcesPanel.jsx` | Memory, CPU and process limits, changed on the running sandbox |
| `SshAccessSection.jsx` | Operator keys and temporary user name/password logins, per sandbox |
| `stacks.js` | Grouping the flat page into stacks; a member whose lead is on another page stays a row of its own and names its lead. Test: `node --test stacks.test.js` |
| `CreateSandboxForm.jsx` | Name, pack, node, memory, CPU, PIDs; the node's free numbers beside them (`feat/ui-sandbox-resources`) |
| `useCreateSandbox.js` | Nodes, packs, the draft, and the pre-check; no JSX (`feat/ui-sandbox-resources`) |
| `SandboxDetail.jsx` | Drawer with overview, agents, logs, shell, rules, inference |
| `AgentsTab.jsx` | Agents of one sandbox: list, add, remove with "keep its data" (`feat/ui-sandbox-resources`) |
| `useSandboxAgents.js` | Agents, pack catalog, add and remove; treats `agent.exists` and `agent.not_found` as sentences, not crashes (`feat/ui-sandbox-resources`) |
| `AdminCenter.jsx` | The full-screen Admin Center shell: header, actions, tab strip; owns no tab body |
| `DashboardTab.jsx` | Metrics, top processes, last blocks, MCP servers (`feat/admin-console-shell`) |
| `TrafficTab.jsx` | Live table, filters, "allow this", temporary window (`feat/console-traffic`) |
| `McpTab.jsx` | Servers, tools, calls, visibility per transport (`feat/console-mcp`) |
| `TerminalTab.jsx` | xterm.js over the terminal WebSocket; my tab list is read from and written to the server, so a reload reopens the same shells. `showAll` (on by default) merges everybody's tabs from `presence.tabsAll` with who is in each, `fixedTab` shows one tab alone for a pop-out. A tab menu offers pop-out, the SSH/PuTTY line, "close for me" (my list only, the shell keeps running) and "close for everyone" (`feat/console-terminal`, workbox) |
| `DesktopTab.jsx` | noVNC over the desktop WebSocket; only a sandbox built with the desktop pack has one. "watch only" reopens the stream with `?view=1` so the server drops the input, `fullWindow` fills a pop-out. Offers to attach the `chrome-browser` MCP server |
| `CommandsTab.jsx` | New processes with their way in (terminal, ssh, inside), polled every 2 s while open |
| `ConnectionsTab.jsx` | Sockets and owning processes, filtered by where they go (internet, SSH, name lookups), polled every 2.5 s |
| `ChatsTab.jsx`, `chats.js` | Model conversations read from the recorded bodies (Anthropic, OpenAI chat, OpenAI responses; SSE stitched back). Needs an inspecting egress mode |
| `AccessTab.jsx` | Ports, SSH key, remote snippets (`feat/console-access`) |
| `PrivacyTab.jsx` | What the content policy found in this sandbox (dashboard, log with direction and masked value → substitute, policy history), and whether this sandbox's egress mode allows an inline check at all |
| `PoliciesTab.jsx` | Effective rules, deployed state; the egress-mode labels come from `features/workbox/network.js`, so this tab and the user's card name a mode the same way (`feat/console-map-policies`) |
| `ProcessesTab.jsx`, `InventoryTab.jsx`, `LogsTab.jsx` | `docker top`, snapshot, log tail (`feat/console-inventory`) |

## The drawer's Agents tab

The drawer (`SandboxDetail.jsx`) answers the quick questions and owns its own tab
list; the Admin Center's tab list is untouched by this package. "Agents" reads
`GET /sandboxes/{id}/agents`, adds one with `POST …/agents` from the catalog at
`GET /agent-packs`, and removes one with `DELETE …/agents/{instance}` carrying
`keep_persistent`. It also shows the generated entrypoint. After an add the
sentence does not claim the agent is up: a running container keeps the command it
was created with until it is recreated.

## The Admin Center contract

`AdminCenter.jsx` is the shell and belongs to the integrator: header, lifecycle
actions, tab strip, and the fetch of the sandbox row. A tab file belongs to one
package and receives:

- `props.sandbox` — the stored row (name, pack, image, status, runtime facts);
- `props.revision` — bumped by `reload`, for a tab that wants to refetch;
- `props.reload` — re-read the sandbox after an action.

A tab never edits the shell. That is what lets seven console packages run at
once and is checked by review, not by a test: the tab list in `AdminCenter.jsx`
is the one place a tab is registered.

Until its package lands, a tab renders `PendingPanel` and says what belongs
there, which endpoint it will read, and who owns it. An empty box looks broken;
sample data looks finished. Both lie.

## The shell tab

A typed line runs as `bash -lc <line>` through `POST /sandboxes/{id}/exec`. It
is request/response, not a PTY: the output comes back when the command ends,
capped at 64 KiB, with the exit code. That is enough to ask Hermes a question
or inspect a file, and it keeps every run in the audit trail as an argument
vector. An interactive terminal is a later transport, not a change to this one.

`MountEditor.jsx` (F15) and `CreateSandboxWizard.jsx` (F24) are placeholders created in Wave 0.

F21: `SandboxListPage.jsx` checkboxes now write to `ui.selection` (kind `sandbox`); the old inline bulk buttons are gone, `BulkBar` (via `BulkHost`) replaces them.
