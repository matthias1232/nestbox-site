# features/inventory/ — data center, site and node pages (F06)

Pages reached from the sidebar tree. Each has a breadcrumb, a title with status, a sticky action
bar and tabs; the active tab is `?tab=<id>` (Summary is the bare address). Members see Summary and
Sandboxes only; the API refuses the rest regardless.

| File | Responsibility |
|---|---|
| `DataCenterPage.jsx` | `/dc/<id>`: Summary (tiles, topology, health), Sites & nodes, Sandboxes, Models, Policies, Permissions, Usage; makes the data center the active environment when it is another reachable one |
| `SitePage.jsx` | `/site/<id>`: Summary (node cards), Nodes, Sandboxes; exports `NodesTable` |
| `NodePage.jsx` | `/node/<id>`: Deploy model here (the single primary), Probe, Drain…, Egress lock, Delete node (danger); tabs Summary, Sandboxes (count of `summary.sandboxes.total`), Models (count of `summary.installs.length`), Images, Traffic, Logs |
| `NodeSummaryTab.jsx` | Gauges (CPU, RAM, VRAM, disk) in one grid row of four cards: the reading under each arc is mono (`41 / 64 GiB`, `23 %`) and the label under it is `--fg-muted`; a gauge whose machine reports no total (VRAM today) reads `–`. Then tiles, tunnel card and facts of a node. Every fact reads a key of `GET /nodes/{id}/summary` as the box returns it: `node.kind`, `node.runner_url`, `node.container_gpu`, `node.last_seen`, `node.created_at`, `site.*`, `data_center.*` (X09 — no `overlay_ip`, `gpu_name` or `joined_at`) |
| `ScopedSandboxesTab.jsx` | Sandbox table with Open / Workbox; selection goes to `ui.selection` (bulk bar) |
| `ScopedModelsTab.jsx` | Model installs of one node or the whole data center |
| `useNodeSummary.js` | `useNodeSummary` (5 s poll of `nodeOps.summary`) and `useTabState` (tab from props/address, writes `?tab=`) |
| `inventoryTabs.js` | Pure helpers: tab lists per kind and role, tab address, size and age formats, tree lookups |
| `inventoryTabs.test.js` | Tests for the helpers above |

Styles: `styles/inventory.css`.
