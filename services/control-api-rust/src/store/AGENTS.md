# store/ — persistence

## Purpose

SQL and the mapping between rows and domain types. Nothing else.

A file here knows how to read and write one aggregate. It does not decide whether
a write is allowed, and it does not know about HTTP. That separation is what lets
a rule change without touching SQL, and a query change without touching a rule.

## Dependencies

- May use: `sqlx`, `domain`
- Must not use: `axum`, business rules

## Security rules

1. Every query uses bound parameters. A value never becomes SQL text. The only
   interpolated identifiers are column names from a closed Rust enum
   (`SortField::column`), never from input.
2. Every tenant-scoped query runs inside a transaction that has set
   `SET LOCAL app.environment_id`. The `TenantScope` helper is the only way to
   get a connection for tenant data, so the scope cannot be forgotten.
3. No `SELECT *`. Columns are named, so a schema change is a compile-time
   mismatch in the mapping instead of a silent field swap.
4. An update checks the version and returns `VersionConflict` rather than
   overwriting a concurrent change.
5. Row contents are never logged. A row may contain a user-supplied string, and
   a log line is a place it does not belong.

## Why runtime queries

`sqlx::query_as` with `FromRow` rather than the compile-time `query!` macros.
The macros need a live database at build time, so the Docker build would need
database access, which is a poor trade. The security property is identical;
what changes is that a schema mismatch is caught by the integration test rather
than by the compiler.

## Files

| File | Responsibility |
|---|---|
| `mod.rs` | `TenantScope`, shared helpers, re-exports |
| `postgres/sandboxes.rs` | Sandbox reads and writes |
| `postgres/policies.rs` | Policy documents, versions, rules, and the compiled rule-set hash |
| `postgres/telemetry.rs` | Event insert and query, batch insert, node identity for ingest, MCP tool-call rows, partition maintenance |
| `postgres/gateways.rs` | Gateways and supervisors |
| `postgres/models.rs` | Catalog (with SQL filters and facets) and hardware profiles per node |
| `postgres/model_installs.rs` | Installs per node and their reservations |
| `postgres/nodes.rs` | Nodes (global rows), sandbox counts per node |
| `postgres/agent_packs.rs` | Baselines, agent packs, tool catalog |
| `postgres/recipes.rs` | Recipes and build outcomes |
| `postgres/persistent_roots.rs` | Roots, mounts, sandbox agents |

Mounts have no `environment_id` column of their own: they hang off a root and a
sandbox. Every mount query therefore carries the tenant either through
`persistent_roots.environment_id` or through the sandbox join, and the row-level
security policy on `mounts` repeats the sandbox check.
| `postgres/jobs.rs` | Jobs, targets, schedules |
| `postgres/auth.rs` | Users, sessions, scopes |
| `postgres/environments.rs` | Environments and clusters |
| `postgres/providers.rs` | Provider profiles and secret references |
| `postgres/audit.rs` | Append-only audit writes |
| `postgres/metrics.rs` | Samples, rollup, partition maintenance, the inventory snapshot |
| `postgres/access.rs` | SSH keys, port pool, inventory, sessions |
| `postgres/egress.rs` | Compiled policy hash, decisions, windows, the node token locator, and the deployment comparison |
| `postgres/mcp.rs` | MCP registry, attachments, tool calls |
| `postgres/traffic.rs` | Traffic rows, map edges, per-destination counts |
| `postgres/traffic_bodies.rs` | Redacted bodies, capped and short-lived: insert per batch, read one, purge by age |

## Unscoped reads, and why each one is allowed

Two categories exist, and neither is a shortcut:

1. **Global tables.** `nodes` and `policy_compilations` are not tenant-scoped by
   design (a machine is shared by every environment). The node token locator and
   the reported policy hash are read and written there without a scope.
2. **The scope itself.** `egress::sandbox_scope` answers "which environment is
   this sandbox in" before a scope can be set. It walks the environments table
   (no row-level policy, by design) and asks each candidate inside a scoped
   transaction. The alternative — reading `sandboxes` without a scope — returns
   nothing for the application role, and reaching for a privileged role to make
   it work is how a tenant filter becomes optional.

## Extending this module

Add the query in the file of the aggregate it belongs to. Take a
`&mut TenantScope`, not a `&PgPool`: that is the compile-time reminder that the
tenant is set. Name the columns. Map into a domain type rather than returning a
tuple, so the service layer never sees a row shape.

The one deliberate exception is `mcp::insert_call`: an MCP call is reported by
the proxy, which speaks for many environments at once, so its writer takes the
pool and trusts the event's own `environment_id` — exactly like
`telemetry::insert`. It still redacts the arguments and bounds the result before
the row exists, because a write path that relies on its caller is a write path
that will be called without the check.

`agent_auth.rs` and `presence.rs` are the workbox's tables; `node_gateway.rs` the tunnel and lock of
a node. The rest of the workbox's reads live in the module of the table they touch.
