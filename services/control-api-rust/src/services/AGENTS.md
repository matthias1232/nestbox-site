# services/ — business rules

## Purpose

The rules: what is allowed, what happens next, and in which order. A service takes
a principal, applies the permission check, calls the store, and returns a domain
value.

This layer exists so a rule has exactly one home. In the prototype a rule lived
partly in the handler and partly in the "state" module, and changing it meant
reading both.

## Dependencies

- May use: `domain`, `store` (through `TenantScope`), `runtime`, `events`,
  `security`, `error`
- Must not use: `axum::*`, HTTP extractors, `sqlx::*`

## Security rules

1. **The permission check lives here, not in the router.** A new route that
   forgets the check is the common failure, and a router-level check is the
   second line rather than the first.
2. Every function takes a `&Principal` and derives the environment from it. A
   caller cannot pass a different environment id.
3. A state change that can fail partway is a job, not a synchronous write.
4. Anything written to the audit trail goes through the store's `record`, which
   redacts.

## Conventions

- A function name is a verb phrase: `start_sandbox`, not `sandbox_start`.
- A service returns domain types or `ApiError`. It never returns a row.
- Rules that are pure (may this status transition?) live in `domain` so they can
  be unit-tested without a database. The service orchestrates.

## The egress decider

`policy_service::check` is the one place that answers whether an outbound request
from a sandbox may proceed. The pure rule (`domain::egress::decide`) stays in the
domain; the service loads what it needs and writes what it decided. Four rules
shape it:

1. **The caller is a node, not a user.** `authenticate_node` runs first, and
   `check` is never reachable without it. A node may only ask about a sandbox
   that runs on that node; the sandbox's `node_id` is compared to the asking
   node, and a mismatch is a deny.
2. **The tenant comes from the sandbox.** A node request carries no environment,
   so `store::postgres::egress::sandbox_scope` resolves it before a scope exists.
   That lookup walks the environments table and asks each candidate through a
   scoped transaction — no privileged read, so row-level security stays in force.
3. **One transaction, one truth.** Rules, windows, the compilation record, and
   the evidence row are read and written together. The row an operator sees later
   therefore describes the rule set that was actually applied.
4. **Fail closed, and say why.** A request without a sandbox, for an unknown
   sandbox, or from the wrong node is a deny with `default_deny`, an empty policy
   hash, and a cache TTL of zero; a database failure is a 5xx, which the proxy's
   `ext_authz` filter turns into a refusal on its own.

Opening or closing a window is a control change: `open_window` publishes
`SandboxControlChanged { what: "window" }` and pushes the OpenShell policy in the
background, because an `openshell` window only reaches the supervisor through
that push (ADR 0053).

## Files

| File | Responsibility |
|---|---|
| `auth_service.rs` | Login, logout, session validation, principal building |
| `access_control.rs` | Permission evaluation and the refusal message |
| `sandbox_service.rs` | Lifecycle, placement, bulk actions, and the proxy environment every sandbox receives |
| `provider_service.rs` | Provider profiles with their probe status, the pending profile an install creates (by container name locally, by overlay address on a joined node), the in-use check on delete, and the reachable-only inference environment a sandbox receives |
| `agent_service.rs` | Agents inside a sandbox: which packs may join, homes and their persistence, the supervisor command |
| `policy_service.rs` | Validation, diff, scope rules, global override, and the egress decider (`check`: windows, rules, one evidence row; `authenticate_node`; `deployment`) |
| `telemetry_service.rs` | Ingest (one event, and the proxy's access-log batch with bodies and MCP calls, including the `would_deny` verdict), live publish, history queries |
| `traffic_service.rs` | Traffic rows with window counts, per sandbox and for the whole environment (the Live traffic page), the map, one body, MCP calls, effective rules with the deployment comparison, live edges for the cluster graph (`feat/live-traffic`) |
| `geography_service.rs` | RDAP lookup per destination host, cached; runs after a traffic read and never blocks it |
| `model_service.rs` | Hardware probing through a node's runner (CPU, RAM, GPU, unified pool), fit against a node's free budget, facets, catalog filters, installs and their reservations (fetched and served on the install's own node, profile corrected to the published address), editing an install (reserved RAM, on/off; off frees the memory) |
| `node_service.rs` | The nodes list, one node with its budget, a node's sandboxes, name-or-id resolution, display rename |
| `pack_service.rs` | Baselines, agent packs, and tool catalog as data; pack defaults by row, baseline compatibility, package-name resolution |
| `builder_service.rs` | Recipes: validation against the catalogs, the Dockerfile generator, content hash, build job, package verification, instantiate with MCP attachments, and `recreate_in_place` for an existing recipe sandbox (a move) |
| `catalogue_service.rs` | The live package catalogue: permission, baseline resolution, fragment validation, delegation to the runtime |
| `template_service.rs` | Image templates: sharing and default rules, validation, apply-to-recipe |
| `selection.rs` | The one validation for package names, repository URLs/keys and template names |
| `gateway_service.rs` | Registry, settings, drift, sync |
| `job_service.rs` | Job creation and progress |
| `catalog_sync/` | The model catalogue is rebuilt from Hugging Face, Ollama, NGC, and NIM, never over a curated row |
| `schedule_service.rs` | Schedule evaluation and runs |
| `metrics_service.rs` | Sample ingest, ranged query, rollup, partition maintenance, the two metrics jobs; `desktop_target` decides the desktop permission (drive needs `SandboxExec`, view only needs `SandboxRead`) and whether the stream really is view-only |
| `inventory_service.rs` | Snapshot storage and age, the allow-list that keeps a value out, never an environment value |
| `access_service.rs` | SSH key pair as a file with a locator, port pool, sessions; pushes the port set and key to the runner and records what it opened |
| `stack_topology_service.rs` | Reads the facts for the stack diagram (timeouts, degrade to unknown) and the appliance components |
| `mcp_service.rs` | Registry validation, attachments, discovery from inside the sandbox, and the agent-config rewrite an attach/detach triggers |
| `media_service.rs` | Media settings, the effective model per sandbox, the provider call, renders and their assets |
| `persist_service.rs` | Persistent roots: derived host paths, mounts, the one-read-write rule, honest usage |
| `terminal_service.rs` | The browser terminal: permission, the four-per-sandbox and twenty-per-user limits, the idle clock, the session row and its two audit events, and the stored tab list a reload reopens (`feat/terminal`) |
| `session_service.rs` | The login overview of running sandboxes: the in-sandbox probe, the assessed login, and per agent the chosen mode with `signed_in` |

## Probing a node

The hardware probe runs on the node, through its runner: `ctx.runtime.hardware()`
returns raw `/proc` and `nvidia-smi` text, and the parsers in `model_service.rs`
turn it into a `HardwareProfile` for that node's id. Measuring from the control
plane would describe the control plane's container and call it the machine.

Three rules follow from that, and all are in `model_service.rs`:

1. **A failed probe invents nothing.** No profile is written, the node is marked
   offline (runner unreachable) or unknown (runner answered with an error), and
   the previous measurement, if any, is returned unchanged. With no earlier
   measurement the answer is `503 node.runtime_unavailable`.
2. **A card is not capacity until a container can have it.** `container_gpu` is
   measured by the runner, stored on the node, and `usable_hardware` clears the
   GPU fields for the fit when it is false.
3. **The runner call happens with no transaction open.** `resolve_node_for` and
   `load_hardware` each open a short scope, so a runner that never answers cannot
   hold a transaction for its timeout. The scope that reads installs and the
   catalog begins afterwards.

Phase 1 has one runtime adapter, so only the local node is measured this way. A
remote node without an adapter gets `unmeasured_hardware` — zeroes and a note,
never a guess about a machine the control plane cannot see.

## The proxy environment

A sandbox has no route out: the runtime network is `internal`, so it has a route
to its own subnet and nothing else, not even a DNS answer for a public name. The
one address it can use is the node's egress proxy, so `create` writes that
address into the sandbox environment (`egress_environment`) and records it in the
sandbox spec, which is what makes "what was this sandbox told" answerable later.

Four decisions worth keeping:

1. **`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY` are set in both spellings.** curl
   and wget read the lowercase ones, Go, Rust and npm read the uppercase ones.
   One spelling is the classic "works in the terminal, not in the agent" bug,
   and the failure looks like a network fault rather than a missing variable.
2. **`NO_PROXY` names the proxy itself**, otherwise a proxy-aware tool sends the
   request for the proxy to the proxy.
3. **The environment is a convenience, not a permission.** A request still needs
   a `network_rules` match; the control API decides per request (ADR 0013). A
   sandbox that ignores the variables is not less governed, it is simply unable
   to reach anything.
4. **The URL comes from `NESTBOX_EGRESS_PROXY_URL`, defaulting to
   `http://egress-proxy:3128`.** `nodes.egress_proxy_url` exists in the schema
   (migration 0022) but not yet on the `Node` domain type; one value a deploy can
   set beats a second source of truth, and when the field reaches the domain type
   only `egress_proxy_url()` changes. The default has to match the runner's
   `NESTBOX_EGRESS_PROXY_HOST`, because the runner is what attaches the proxy to
   the sandbox network under that alias.

`builder_service::instantiate` calls the same two functions. A recipe sandbox is
created through a different path, and a path that forgot the proxy would be a
sandbox that is isolated and therefore useless, with nothing in the UI saying
why.

## Trust files, agent files, and where a sign-in mode acts

The container is built once and never gains a mount, so everything a later mode
switch could need is present from the start:

1. **Both CA files, mounted by the runner, referenced by `create`/`recreate`.**
   `EGRESS_CA_IN_SANDBOX` is the CA alone and is what `NODE_EXTRA_CA_CERTS` gets;
   `CA_BUNDLE_IN_SANDBOX` is the public roots plus that CA and is what the clients
   which *replace* their store get (`SSL_CERT_FILE`, `REQUESTS_CA_BUNDLE`,
   `CURL_CA_BUNDLE`, `GIT_SSL_CAINFO`, `AWS_CA_BUNDLE`). `egress_environment_for`
   adds the trust variables for every mode with `uses_proxy()`, not only for the
   modes that read TLS today, so switching to `transparent` needs no recreate.
2. **The container keeps the gateway keys.** A sign-in mode is a launcher's
   decision (ADR 0054): `write_agent_files` strips the gateway settings from the
   map that feeds the *config files*, and the launcher strips the variables for a
   `subscription` agent. `apply_login_choice` is for config files only.
3. **`sandbox_service::write_agent_files` is the one place that writes agent
   files**, for create, recreate, `reconfigure_pack` and a mode switch. It reads
   the sandbox, its egress mode and every pack it holds (the pack's own plus one
   per recipe agent), builds the environment from `provider_service`, strips by
   mode, adds the proxy variables, writes each pack's files with the MCP servers,
   and installs the launcher. It returns `Ok(0)` for a sandbox that is not
   running: the files are written the next time it runs.
4. **The spec, not the request, is what a recreate reproduces.** `command`,
   `gpu_count`, `memory_reservation` and `cpu_shares` are stored at create and
   read back in `recreate`. A GPU request on a node with `container_gpu == false`
   is a 409 before a row exists.
5. **Sign-in rules come from one place.** `create` calls
   `agent_auth_service::sync_login_rules` after the commit instead of inserting
   `Sign-in: <host>` rules itself, so a mode switch and a create cannot disagree.
6. **Model readiness is spawned, not awaited**, after create, recreate and a
   successful start.

## Persistence rules

1. A mount source is never taken from a request. `persist_service` derives the
   host path from a validated slug plus the caller's user id, and that derived
   path is the only source the runtime ever receives.
2. `sandbox_service::create` reads a sandbox's mounts from the database, inside
   the transaction that inserted the sandbox row, and passes them to the runtime.
3. A bind mount is fixed when the container is created. A mount or dismount
   changes the row, and therefore what the *next* container for that sandbox
   receives; it does not rewire a running container.
4. Archive, never delete, by default. Purging removes the record only: the
   directory lives on the node, which the control plane cannot reach.

## The proxy's telemetry batch

`telemetry_service::ingest_batch` is the one write path that takes a tenant from
the request body instead of from a principal, because the caller is a node, not a
user. That inversion is why the checks around it are explicit:

1. **The node token is resolved, not compared to a constant.** The node row's
   `node_token_ref` is a locator; `security::secrets::resolve` reads it and the
   comparison is constant-time over SHA-256 hashes. No header, unknown node, no
   locator, unreadable or permissive file, and a mismatch are all the same 401 —
   a probe learns nothing from the difference.
2. **The environment is validated, and every named sandbox must be in it.** A
   batch that names another tenant's sandbox is refused whole; a partial write
   would put one tenant's traffic in another tenant's timeline.
3. **Limits are rules, not conventions.** 500 events per batch, 64 KiB per body
   side (cut and flagged `truncated`), 4 MiB of body text per batch. An oversized
   batch is refused before the first row.
4. **Redaction is in the store.** Events, bodies, MCP arguments, and MCP results
   all pass `security::redaction` on the way to the database, so a new caller
   cannot store a body verbatim by forgetting a call.
5. **Ids are derived from the request id.** A retried batch collides on
   `(id, ts)` and is skipped, so a timeout does not double a timeline.

## The builder

`builder_service.rs` deserves a note because its decisions are not obvious from
the signatures.

- **The Dockerfile is generated, never stored.** The recipe holds the choices;
  the catalogs hold what those choices mean; the generator is the only place the
  two meet. A corrected pack row changes the next build with no migration.
- **A build is a job that runs detached.** The endpoint records a `recipe_build`
  job, marks the recipe `building`, and returns. The task is spawned in process
  because there is no queue worker yet; a restart during a build leaves the row
  `building`, and a PATCH recovers it. The job body is `builder_service::run_build`,
  public so a test can run it without a detached task.
- **`instantiate` writes the sandbox row itself** rather than calling
  `sandbox_service::create`, because a recipe carries an agent set and a
  content-addressed image tag that `CreateSandbox` cannot express, and its
  mounts have to be in the spec the container is created with. It reuses the
  same pieces (`SandboxName`, the placement rules, `pack_needs_inference`, the
  status settlement), so the two paths send the same shape to the runtime. Once
  `CreateSandbox` can carry agents and a recipe image, this should collapse into
  a call to it.
- **The catalogs are read through the store, not `pack_service`.** Both read the
  same rows; going through `pack_service` is the cleaner layering and changes
  nothing about the recipes.

## The MCP layer

`mcp_service.rs` keeps three decisions that look small and are not.

1. **Discovery runs inside the sandbox.** `tools/list` is an outbound request,
   so it is made by a probe executed through `ctx.runtime.exec`, not by the
   control plane. The control plane is on the host network: a request from here
   would bypass the sandbox's proxy, its rules, and the identity the policy is
   written against. A discovery path that skipped the rule check would be an
   egress hole with a friendly name (ADR 0026).
2. **A failed discovery is an attachment state, not a 5xx.** The row becomes
   `status=error` with `last_error`, and the response is the row. Request errors
   stay request errors: unknown sandbox 404, stopped sandbox 409, `stdio` server
   422. Returning 5xx would leave the row claiming the previous, optimistic
   state, which is the failure mode the column exists to prevent.
3. **Transport is validated as a pair with endpoint/command.** A `stdio` server
   with an endpoint, an `http` server without one, or a stdio server claiming
   `oauth` is a 422 naming the combination. The database CHECK constraints agree,
   but they cannot produce a sentence a user can act on.

A credential never enters this layer: `env_slots` holds variable **names**, the
probe reads the value inside the container, and `mcp_tool_calls` arguments and
results are redacted in the store on the way in.

The probe speaks the `2025-06-18` handshake (`initialize`, then `tools/list`).
A server that only offers the stateless `2026-07-28` discovery is recorded as an
error naming its refusal; it is not silently retried with a second protocol,
because two handshakes would make "discovery failed" ambiguous. Extend the probe
when there is a server that needs it, not before.

## The catalogue and templates

`catalogue_service.rs` asks the runtime, which asks a container of the baseline
image. It keeps no cache: the runner's ten-minute cache is next to the index it
talked to, and a second cache would be a second staleness to reason about.

`template_service.rs` keeps the rule that a template carries choices, never an
image. Applying one copies fields onto a recipe, moves the revision and clears
`image_ref`, because the previous image does not contain the new selection.

## Extending this module

Add the rule here first, as a pure function in `domain` if possible, and only
then wire it to the store. If the rule needs a permission, add the permission to
`domain::auth` before writing the check, so the check cannot reference a
permission that does not exist.

## The terminal

`terminal_service.rs` splits a session in two on purpose: `open` authorises and
reserves a row, `attach` runs the PTY. The split is what lets a refusal be an
HTTP status code — a WebSocket that upgrades and then closes explains nothing —
and it is why the transport is two small ports (`ClientSocket` for the browser,
`runtime::TerminalSocket` for the PTY) instead of an axum type in this layer.

The idle clock measures input, not output (ADR 0023). The limits are counted
from the open rows, so they survive a restart; the advisory lock in
`terminal_sessions::lock_user` makes the count and the insert one decision.
Nothing in this file may write a keystroke or a line of output anywhere.

The audit detail names the session `terminal_id`, not `session_id`: the central
redactor masks every key containing "session" (so a session cookie can never
leak), and a masked reference reads like a redacted secret instead of a pointer.
The row itself is in `sandbox_sessions`; `terminal_id` is the join key a reviewer
uses to find it.

## Multi-tenant and multi-node services (ADR 0044-0050)

| File | Responsibility |
|---|---|
| `tenancy_service.rs` | Customers, data centers, sites, sandbox grants, invitations |
| `oidc_service.rs` | Sign-in through an identity provider: PKCE flow, profile parsing, who a stranger is |
| `node_join_service.rs` | Join tokens, the join script, overlay addresses, the hub's peer list, the sync loop |
| `node_join/` | The scripts and the compose file a joined node runs (embedded with `include_str!`): `prepare.sh` installs and makes the key, `apply.sh` brings up the tunnel, the egress lock and the node package, `join.sh` is both with the join request in the middle |
| `node_join_service.rs` | Join tokens, the join script, overlay addresses, the hub's peer list, the sync loop; `join_with_key` also stores the endpoint the hub dials |
| `node_join/` | The three scripts (`join.sh`, `prepare.sh`, `apply.sh`) and the compose file a joined node runs (embedded with `include_str!`) |
| `image_sync_service.rs` | Image assignments, the images x nodes matrix, the pull loop |
| `move_service.rs` | Moving a sandbox and its folders to another node with restic snapshots; the preflight that says whether it can work, the rollback after a failed cutover, and the model warm-up on the target |
| `provision_service.rs` | Provider accounts, renting machines, the budget, the reaper |

## Workbox control (ADR 0052-0057)

| Service | Question it answers |
|---|---|
| `agent_auth_service` | which agent signs in how, how a change reaches a running agent, the sign-in rules that follow the mode, and the restart that waits for an idle agent |
| `agent_launch_service` | the launcher, agentctl and env.sh written into a sandbox |
| `agent_activity_service` | which tmux panes run an agent, and whether one is busy |
| `egress_live_service` | switching a sandbox's proxy mode without a new container, the revert timer that switches it back (`revert_plan`, `revert_due`), and the `direct` exception that still rebuilds |
| `agent_activity_service` | which tmux panes run an agent, whether one is busy, restarting a pane or a supervised agent, recent model traffic |
| `egress_live_service` | switching a sandbox's proxy mode without a new container |
| `presence_service` | who is connected to a sandbox now |
| `model_grant_service` | which models a sandbox may use, and where they are served; `provider_service::environment_for_sandbox` then points the agent at the accounting router (`NESTBOX_MODEL_*`, `…/s/<sandbox>/r`) |
| `model_readiness_service` | starting and warming those models before they are used |
| `node_connect_service` | joining a machine by address (prepare, join, apply over SSH, credentials never stored), tunnels from the hub's `status.tsv`, the egress lock |
| `rule_bulk_service` | turning observation into rule drafts |
| `node_connect_service` | joining a machine by address, tunnels, the egress lock |
| `rule_bulk_service` | turning observation into rule drafts (one allow draft per denied or would-deny destination) |
| `control_service` | the one summary the workbox and the admin list read |

`agent_launch_service` installs the launcher with one `sh -c` per sandbox: the scripts arrive
base64-encoded, so no shell quoting of the payload is needed, and every file is written beside its
target and moved into place, so a shell never sources half a script. A mode change does not
reinstall anything: the launcher reads `.nestbox/mode/<agent>` at start, and `write_mode` writes
that one line.
`policy_service::sync_openshell_policies` pushes each sandbox's compiled policy and, while the rules
are suspended (`off_allow_all`, or an open `all`/`openshell` window), the destinations it has been
seen reaching, in audit mode: OpenShell logs them instead of blocking.
| `chat_service.rs` | Chat turns: storing, querying, grouping, and who may read the text |
| `chat_live.rs` | The live report of a running model request; nothing stored |
| `privacy_exceptions.rs` | DLP exceptions: create, list, delete, and the check the filter asks |
| `quick_allow.rs` | Allow one destination for a while, from the live view |

Control Center services (Wave 0 stubs returning `ApiError::internal("not built: <package>")`; a package replaces the body in place):

| File | Responsibility |
|---|---|
| `inventory_tree_service.rs` | `tree` (B01) |
| `search_service.rs` | `search` (B02) |
| `bulk_service.rs` | `sandboxes` (B03) |
| `model_bulk_service.rs` | `install_bulk` (B04) |
| `model_cluster_service.rs` | `list get create update delete add_member remove_member proxy_view` (B05): a cluster plus its `local_openai_compatible` provider profile in one transaction; `proxy_view` reads the appliance data center |
| `node_ops_service.rs` | `summary drain quick` (B07) |
| `alert_service.rs` | `list`, `publish_if_refused` (B08; the latter is a no-op until then) |
| `mount_service.rs` | `effective set` (B09) |
| `policy_preview_service.rs` | `preview compiled` (B10) |
| `quick_control_service.rs` | `set` (B11) |
| `permission_service.rs` | `list set remove` (B12) |
| `tour_service.rs` | `mark_seen` (F18) |
