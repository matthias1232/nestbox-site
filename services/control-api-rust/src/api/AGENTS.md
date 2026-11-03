# api/ — HTTP

## Purpose

Read the request, check what it claims, delegate to a service, format the answer.

Nothing here contains a rule or a query. That is not stylistic: a handler that
queries the database is a handler that can bypass the tenant scope and the
permission check, and it is the exact shape that dissolved the previous version
of this service.

## Dependencies

- May use: `services`, `domain`, `error`, `security`, `state`
- Must not use: `sqlx::*`, `store::*` directly, `runtime::*` directly

The architecture test enforces the last two.

## Security rules

1. Identity comes from the `Auth` extractor, which is the only place a credential
   is read. CSRF is enforced there too, for session requests only.
2. A handler never trusts a body to name its own environment or actor; both come
   from the principal.
3. A path parameter is parsed into a typed value (`Uuid`) by the extractor, so a
   malformed id is a 400 before any logic runs.
4. An id supplied in a create body is ignored where the database assigns one.
5. Either credential — a session cookie or an API token — may name the data
   center it works in with `x-nestbox-environment`. The extractor hands the id to
   `auth_service::principal_for_environment`, which checks membership, so the
   header can never widen what a credential reaches: a data center without a role
   is a 403 and an unknown id a 404. Without the header nothing changes, and a
   malformed header is ignored rather than refused, because a request that would
   have worked must not fail over an optional hint. The acceptance script relies
   on this: it sets the header and asserts `/auth/me.environment_id`, so a run
   cannot silently grade an empty data center.

## Files

| File | Routes |
|---|---|
| `mod.rs` | Router composition and layers |
| `principal.rs` | The `Auth` extractor: session, token, CSRF, `x-nestbox-environment` |
| `auth.rs` | Login, logout, identity |
| `system.rs` | Health, readiness (database state plus `disk_free_bytes` from the host load, `null` when the runner does not answer within 2 s; the status code never depends on the disk), runtime, OpenAPI, environments, audit |
| `sandboxes.rs` | Sandbox list, detail, create (including the optional `gpu_count`), actions, logs, exec, stack membership (`PUT /sandboxes/{id}/stack`) |
| `sandboxes_settings.rs` | A sandbox's supervisor settings: read the thresholds, write them back |
| `policies.rs` | Policies (with the kind derived from their sections), the revision history, rules, drafts, traffic windows, and the node-token egress check (`feat/policy-decider`) |
| `telemetry.rs` | History, facets, ingest, and the node-token batch from an egress proxy |
| `events.rs` | SSE streams |
| `cluster.rs` | Gateways, supervisors |
| `providers.rs` | Provider profiles, probe |
| `models.rs` | Catalog per node with filters, per-node hardware, facets, requirements, installs with reservations, editing an install (`PATCH /models/{key}/install`), sync |
| `nodes.rs` | Nodes with budget and sandbox count, one node, its sandboxes, display rename, probe through the runner |
| `packs.rs` | Baselines, agent packs, tool catalog with `?baseline=` resolution, and the live package catalogue |
| `recipes.rs` | Recipe CRUD, build, instantiate, apply-template (`feat/recipes-build`) |
| `image_templates.rs` | Image templates: list, default, create, read, replace, delete |
| `persistent_roots.rs` | Roots, mount, dismount, usage |
| `sandbox_agents.rs` | Agents inside a sandbox: list, add, remove with `keep_persistent` |
| `jobs.rs` | Jobs, schedules |
| `metrics.rs` | Resource history, node sample ingest, process table |
| `inventory.rs` | Snapshot and refresh |
| `stack.rs` | `GET /sandboxes/{id}/stack-topology`, `GET /system/components`, component logs |
| `access.rs` | SSH key, one-time download, rotation, ports, sessions, `GET /sandboxes/{id}/connect`, `PUT /nodes/{id}/endpoint` (`feat/sandbox-access`) |
| `traffic.rs` | Traffic rows for one sandbox and for the whole environment, the denied-destination decisions view, one body, map edges, effective policies, MCP calls (`feat/live-traffic`) |
| `mcp.rs` | MCP registry and sandbox attachments, and the discovery trigger |
| `media.rs` | Media settings, a sandbox's choice, renders and assets, and the media-token bridge |
| `sandboxes_terminal.rs` | The terminal WebSocket and the stored tab list a reload reopens (`feat/terminal`) |
| `topology.rs` | The cluster graph and its live edges (`feat/live-traffic`) |

The Admin Center's read endpoints (`traffic.rs`, `topology.rs`) take a window and
return a bounded page. `GET /topology` without `?live=` is the static graph
unchanged; the parameter only appends edges, so a renderer that knows the static
kinds keeps working.

`models::hardware_json` is the one shape for a machine: the catalog envelope,
`GET /models/hardware`, and `POST /nodes/{id}/probe` all use it, so it always
names the node, its status, and whether containers there can have a GPU next to
the measurement. A second shape would let those three drift apart.

## Work packages

A route may be registered and documented before its handler exists: the stub
answers `501 server.not_implemented`. This is how parallel packages share one
contract. A package fills the handler bodies in its file and touches nothing
in `mod.rs` or `openapi.rs`.

## Extending

Add the handler to the file of its resource, then add the route in `mod.rs`
**and** the operation in `openapi.rs`. The OpenAPI test fails if a route is
documented without existing, or the reverse.

## The terminal route

`GET /sandboxes/{id}/terminal` is the only WebSocket in this layer, and it is
still a handler and not a rule:

1. the `Origin` header is checked against `config.origins` and against the
   request's own host, because a WebSocket is not covered by the same-origin
   policy and a session cookie is. A request without an `Origin` is accepted only
   for a bearer token;
2. `terminal_service::open` runs **before** the upgrade, so a missing permission,
   a stopped sandbox, or a full quota is a 403, a 409, or a 429 with a message;
3. after `on_upgrade`, `BrowserSocket` adapts axum's messages to the service's
   `ClientFrame`/`ServerFrame` port, and `attach` runs the session.

The close frame the console shows comes from the service; the handler only adds
the WebSocket close. A refusal that happens before the upgrade is a normal HTTP
error with the published envelope, which is why the terminal route is the one
place in this layer where a quota is a status code rather than a frame.

`sandboxes_vnc.rs` is the desktop route. It follows the terminal's rules: the
origin is checked first, the permission and the sandbox state before the
upgrade, and then frames are moved between the browser and the runner without
being read. Driving needs `SandboxExec`; `?view=1` needs only `SandboxRead`, and
a caller without `SandboxExec` is view-only even without it — the runner then
drops all input. Each connection is a `desktop` presence row, opened after the
upgrade and closed when the stream ends; a failure there is logged and never
ends the stream. `GET /sandboxes/{id}/live/{kind}` (in `metrics.rs`) is the read-only
live view (`connections`); the kind is a closed list. `POST
/sandboxes/{id}/allow` creates an enabled allow rule for one destination in one
step (idempotent) — the one-click release from the Traffic tab.

## Multi-tenant and multi-node routes

`tenancy.rs` (organisations, sites, grants, invitations), `oidc.rs` (sign-in providers, callback),
`node_join.rs` (join tokens and the public join routes, node package, moves), `images.rs` (registry,
assignments, matrix), `hosting.rs` (provider accounts, rented machines, cloud-init). The join routes and
the invitation preview are public on purpose: the token in the path is the credential.

## Workbox control routes

`agent_auth.rs`, `control.rs`, `presence.rs`, `rule_bulk.rs`, `model_grants.rs`,
`model_readiness.rs`, `node_connect.rs` and `move_preflight.rs` carry the workbox's endpoints. Each
handler parses, authorises through `Auth`, and delegates; the two internal routes
(`accounting/models/{sandbox}`, `node/egress-lock`) authenticate a service instead — `node/egress-lock`
verifies the node token in the service, with the same check that guards the node package fetch.
| `chats.rs` | Chat turns, groups, one turn, the live-watch audit |
| `chat_live.rs` | The internal live report from the broker and the accounting proxy |
| `privacy_exceptions.rs` | DLP exceptions: list, create, delete |
| `privacy_refusal.rs` | A refusal without a finding, reported by the proxy |
| `quick_allow.rs` | Allow rules with an end |

Control Center routes (Wave 0 stubs; each handler calls a service that answers 500 "not built: <package>" until its package lands):

| File | Routes |
|---|---|
| `inventory_tree.rs` | `GET /inventory/tree` (B01). Named apart from `inventory.rs`, which serves a sandbox's installed software |
| `search.rs` | `GET /search`: parses `kind` list (unknown → 422), calls `search_service` |
| `inventory_tree.rs` | `GET /inventory/tree?view=` (B01): parses the view (422 otherwise) and answers a `TreeResponse`. Named apart from `inventory.rs`, which serves a sandbox's installed software |
| `search.rs` | `GET /search` (B02) |
| `bulk.rs` | `POST /sandboxes/bulk` (B03); passes the shared config for the move action |
| `models.rs` (`install_bulk`) | `POST /models/{key}/install-bulk` (B04) |
| `model_clusters.rs` | `/model-clusters`, members, and the proxy's `GET /internal/accounting/clusters/{id}` (B05) |
| `node_ops.rs` | `GET /nodes/{id}/summary`, `POST /nodes/{id}/drain`, `POST /nodes/quick` (B07); drain and quick pass `state.config` to the service |
| `model_clusters.rs` | `/model-clusters`, members (`install_id`, optional `weight`), and the proxy's `GET /internal/accounting/clusters/{id}` (B05) |
| `node_ops.rs` | `GET /nodes/{id}/summary`, `POST /nodes/{id}/drain`, `POST /nodes/quick` (B07) |
| `alerts.rs` | `GET /alerts` (B08) |
| `node_ops.rs` | `GET /nodes/{id}/summary`, `POST /nodes/{id}/drain`, `POST /nodes/quick` (B07) |
| `alerts.rs` | `GET /alerts?window=&sandbox=` (B08; 422 on a bad window) |
| `mounts.rs` | `GET/PUT /sandboxes/{id}/mounts` (B09) |
| `mounts.rs` | `GET/PUT /sandboxes/{id}/mounts`, thin over `mount_service` (B09) |
| `policy_preview.rs` | `POST /policies/preview`, `GET /policies/{id}/compiled` (B10) |
| `policy_preview.rs` | `POST /policies/preview`, `GET /policies/{id}/compiled` (B10) | (built)
| `quick.rs` | `PUT /sandboxes/{id}/quick` (B11) |
| `permissions.rs` | `GET/PUT/DELETE /permissions` (B12), contract C13 |
| `tours.rs` | `PUT /users/me/tours/{tour}` (F18) |
| `permissions.rs` | `GET/PUT/DELETE /permissions` (B12) |
| `tours.rs` | `PUT /users/me/tours/{tour}`, `GET /users/me/tours` (`list_seen`) |


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `accounting.rs` | Accounting and the internal ingest the proxy calls; the record body keeps the proxy's `detail` object (cluster and member ids) instead of dropping it |
| `backups.rs` | Backups of persistent roots. |
| `cudax.rs` | CUDA-X catalogue and per-node installation endpoints. |
| `privacy.rs` | Content privacy (DLP): the policy, a single filter test, incidents, purge. |
| `proxies.rs` | Exit proxies over HTTP. |
| `secrets.rs` | The secret store: write-only keys. |
| `users.rs` | User administration (admin only). |
