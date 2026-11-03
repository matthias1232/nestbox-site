# store/postgres/ — the PostgreSQL implementation

## Purpose

One file per aggregate, holding the SQL for it. This folder is the only place in
the codebase where SQL appears.

## Security rules specific to this folder

1. Bind every value. The only interpolated text is a column name from
   `SortField::column()`, which is a closed Rust enum.
2. Never `SELECT *` in a read used by the API; name the columns so the mapping
   is checked against the schema by the integration test.
3. An `UPDATE` carries `AND version = $n` and reports `VersionConflict` when it
   matches nothing. Optimistic locking is what makes two operators editing the
   same policy a 409 instead of a lost update.
4. The audit table accepts inserts only. There is no update or delete statement
   for it here, and the application role's grants refuse one anyway.
5. An ingest write sets the tenant scope in the repository, not only in the
   service. `telemetry_events` and `inference_requests` carry row-level
   policies, the API connects as the non-owner `nestbox_app` role, and the
   ingest path is a write that legitimately names its own tenant — so
   `telemetry.rs` opens the scope itself. A test that connects as the owner
   would pass without it and prove nothing.

## Files

| File | Aggregate |
|---|---|
| `mod.rs` | Re-exports and shared row helpers, and `is_missing_function`: whether an error is SQLSTATE 42883, the only case a caller may answer with a fallback for a schema that predates a function (review finding F11) |
| `sandboxes.rs` | Sandbox list, read, insert, status change, delete; `set_telemetry_mute` and `telemetry_mute` (recording mutes, B11). The insert writes `owner_user_id` in the same statement as the row: the policy that lets a member write only what is theirs (migration 0090) has no window for an unowned row |
| `auth.rs` | Users, sessions, API tokens, scopes |
| `policies.rs` | Policy documents, versions, network rules, drafts, the compiled rule-set hash, traffic windows (open, list, revoke, the environment's `max_window_minutes`) |
| `egress.rs` | Compilation hash record, decisions, open windows for the decider, node token locator, deployment comparison |
| `policies.rs` | Policy documents, versions, network rules, drafts, the compiled rule-set hash |
| `egress.rs` | Compilation hash record, decisions, open windows, node token locator, deployment comparison |
| `egress_live.rs` | The tenant list and the due revert timers the live-mode revert loop walks before `policies.rs` writes one mode |
| `telemetry.rs` | Event insert, query, batch insert, ingest's node lookup, MCP tool calls, partition maintenance |
| `gateways.rs` | Gateways, supervisors, sync state |
| `models.rs` | Model catalog, hardware profiles, inference rows, the catalog-sync upsert |
| `model_installs.rs` | One model on one node: reservation, state, provider link, its job |
| `nodes.rs` | Nodes, the local node, seed marks, display fields, sandbox counts per node |
| `node_gateway.rs` | A node's tunnel state and egress lock, the gateway columns a join writes, the credentials a node presents (`nodes` has no policy: unscoped reads) |
| `jobs.rs` | Jobs, targets, schedules; `settle_interrupted` (pool, not scope: the boot has no principal) marks the given kinds' `pending`/`running` rows failed, for the jobs a restart killed |
| `environments.rs` | Environments, clusters, nodes, overview counts |
| `providers.rs` | Provider profiles, secret references; `set_endpoint` corrects the address once a server on a joined node reports where it is published |
| `agent_packs.rs` | Baselines, agent packs, and the tool catalog (global data, no tenant filter) |
| `persistent_roots.rs` | Persistent roots, mounts, and the agents inside a sandbox |
| `recipes.rs` | Recipes, the build outcome, applying a template, and the link from a sandbox to its recipe |
| `image_templates.rs` | Saved builder selections, their sharing rule and the one default per environment |
| `audit.rs` | Append-only audit writes |
| `traffic.rs` | Traffic rows for one sandbox and for a whole environment, window counts, map and live edges, one body read, MCP calls, effective rules, the open windows that cover a sandbox (a window with no end counts), the two hashes behind "deployed", and the denied-or-would-deny decisions list |
| `geography.rs` | `host_geography`: the cached country and AS per destination host (global table, pool not scope) |
| `terminal_sessions.rs` | Terminal sessions in `sandbox_sessions`: open, count for the limits, close. Never content (ADR 0023) |
| `terminal_tabs.rs` | The named terminal tabs one user keeps in one sandbox: list, replace. Never content |
| `login_rules.rs` | The per-sandbox `Sign-in (<agent>): <host>` rules that open an agent's own-account hosts: delete for a sandbox, insert one host, list the environments the restart loop walks |
| `agent_activity.rs` | Whether a sandbox talked to a model in the last 30 seconds, from `telemetry_events` |
| `model_grants.rs` | `model_grants`: list, one row, the grants that apply to a sandbox, insert, delete, the sandboxes a change reaches, and the owner/pack/status facts of one sandbox |

## Reading the traffic log

`traffic.rs` reads the proxy's access log out of `telemetry_events.detail` instead
of a column per field, because the fields are the proxy's and they gain one at a
time. Four consequences, all deliberate:

1. **A malformed value is a `NULL`, not a failed page.** Every numeric field is
   guarded by a regex in SQL before its cast, so one bad line cannot blank a
   list of two hundred.
2. **Three spellings are accepted for a destination** (`host` from the access log,
   `destination` from the older telemetry type, `hostname` from a DNS event,
   which has no connection to name). A row written by any ingest path appears; a
   reader that knew only one would show a gap where there is none. DNS rows are
   read as evidence too (`kind IN ('network','dns')`), because a name lookup is
   traffic and the operator asking "what did this sandbox reach" means both.
3. **A row without a decision is not traffic.** `decide` answers in three words,
   and the first column of the table is that answer; inventing one would make the
   table an opinion.
4. **A destination the rules would have denied is listed too.** A window or an
   allow-all mode lets traffic through while the rules still answer deny; the
   ingest stores that verdict as `detail.would_deny`, and the decisions view
   groups those rows with real denials, flagged `observed_only`.

Counts and edges are grouped in SQL, never assembled from returned rows: a
five-minute window at ten thousand events a minute is a number the browser must
not have to produce. Every statement is bounded by `ts`, `environment_id`,
`sandbox_id`, and `LIMIT`.

One carve-out: the **body read** lives here rather than in `traffic_bodies.rs`,
because the body is shown from the traffic row and the read must be scoped by the
same sandbox. `traffic_bodies.rs` keeps the write side and the retention sweep.


## Extending this folder

Add the statement to the file of its aggregate. Take `&mut TenantScope`. Name
the columns. Map to a domain type. If the query needs a new index, add it in a
migration in the same change, and say in the migration comment which query it
serves.

## Two rows that live with `telemetry.rs` for a reason

- **The node lookup** (`node_identity`) reads `nodes`, whose aggregate file is
  `nodes.rs`. It lives in `telemetry.rs` because it exists only for the ingest
  path's authentication, and moving it would make the ingest dependency look like
  a general one. `nodes` has no environment column and no policy; the query is
  deliberately unscoped and takes the admin pool.
- **The MCP tool-call insert** (`insert_tool_calls`) writes `mcp_tool_calls`,
  whose aggregate file is `mcp.rs`. It lives here because it is the same wire
  observation as the traffic row it hangs off — same event id, same tenant, same
  transaction boundary — while `mcp.rs` owns the server catalogue and what
  discovery found. If the two ever need one transaction, this is the place to
  join them.
| `chat_turns.rs` | Rows of `chat_turns`: insert, query, group |
| `privacy_exceptions.rs` | Rows of `privacy_exceptions` |

Control Center stores (Wave 0): `inventory.rs` (B01), `search.rs` (B02), `model_clusters.rs` (B05), `alerts.rs` (B08: `refused_grouped`),
Control Center stores (Wave 0): `inventory.rs` (B01: `environment_rows`, `placement_rows` as one LEFT JOIN chain whose 15-minute alert count uses the alert list's own grouping and its two answered conditions, `user_rows`, `install_rows`, `cluster_rows`), `search.rs` (B02), `model_clusters.rs` (B05), `alerts.rs` (B08),
`permissions.rs` (B12) return empty results until their package fills the queries; `tours.rs` is complete (the table
Control Center stores (Wave 0): `inventory.rs` (B01), `search.rs` (B02), `model_clusters.rs` (B05), `alerts.rs` (B08),
`permissions.rs` (B12) lists and writes roles per data center, sandbox and organisation; `tours.rs` is complete (the table
`ui_tours_seen` is per user, so it takes the pool, not a tenant scope). Migration `0086_control_center.sql` adds
`model_clusters`, `model_cluster_members`, `sandboxes.telemetry_mute_until`/`telemetry_mute_bodies_until` and `ui_tours_seen`.
`model_clusters.rs` (B05): list/get/create/update/delete, `add_member` (upsert weight), `remove_member`, `members` (joins installs, nodes, provider profiles), `proxy_view` (takes a scope: the tables have RLS), `environment_of` (calls `nb_cluster_environment` on the pool: the one cross-tenant read the accounting proxy needs, migration 0087), `model_exists`, `install_model_key`.

A `traffic_windows` row with `expires_at IS NULL` is an unlimited allow (migration `0078_workbox_control.sql`): no
end date, not an unknown one. Every query that asks whether a window is in force therefore reads
`(expires_at IS NULL OR expires_at > now())`; `alerts.rs` (`refused_grouped`), `inventory.rs`
(`placement_rows`, the tree's alert count) and `traffic.rs` (`open_windows`) all say it that way, and a
test in `tests/control_center.rs` holds them to it.

## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `access.rs` | How an operator reaches a sandbox: SSH keys, published ports, sessions. |
| `accounting.rs` | Accounting rows (including the `detail` jsonb of migration 0087), price list, daily rollup, and retention settings |
| `agent_auth.rs` | Rows of `sandbox_agent_auth`: the chosen sign-in mode per agent of a sandbox. |
| `backups.rs` | The catalogue of root backups. The archives are files on the node; this layer holds |
| `control.rs` | The workbox's read models: every sandbox's controls at a glance (ADR 0053). |
| `cudax.rs` | CUDA-X catalog and per-node installations. |
| `media.rs` | Media settings per environment and per sandbox, the render record, and `sandbox_environments_by_name`: the data centers a sandbox name appears in, from `nb_sandbox_environment(name)` (migration 0088), which is how the accounting proxy finds a sandbox's tenant without a principal |
| `metrics.rs` | Sandbox resource samples, their rollup, and the inventory snapshot. |
| `model_metrics.rs` | Samples of what a served model reports about itself. |
| `model_readiness.rs` | The readiness reads: which profile serves a granted model, and on which node. |
| `presence.rs` | Open sessions (`sandbox_sessions`) as presence: who is in which terminal tab |
| `privacy.rs` | Privacy settings and incidents (`0056_privacy_layer.sql`). |
| `proxies.rs` | The exit proxy catalogue. |
| `vault.rs` | Stored secrets: ciphertext in, metadata out. The value never leaves this layer |
