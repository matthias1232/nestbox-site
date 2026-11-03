# tests/ — architecture and integration checks

## Purpose

Two different jobs:

- `architecture.rs` enforces the layer boundaries by reading the source tree.
  A violation fails the build, which is the only way the rule survives.
- The integration suites in the API exercise real behaviour against a real
  PostgreSQL, because the migration path, row-level security, and cursor paging
  are exactly the things an in-memory fake would not test.

## Rules

- An architecture test asserts a property that a reviewer would otherwise have
  to remember.
- An integration test names the behaviour, not the function: "a viewer cannot
  delete a sandbox" rather than "test_delete_403".
- A test that needs a database skips cleanly when `TEST_DATABASE_URL` is unset,
  so `cargo test` works on a laptop without one.
- A suite that needs a *migrated* database creates its own database from
  `TEST_DATABASE_URL` rather than dropping `public` in the shared one, so two
  suites can run in the same `cargo test` without deleting each other's schema.
  `recipes.rs` is the pattern.
- `policy_preview.rs` previews a policy against database `nestbox_cc_b10` (B10).
- `policy_bulk.rs`, `tenancy.rs`, `backups.rs`, `stack_topology.rs` and
  `multitenant.rs` each build a `CreateSandbox`; a field added there is one line
  in every one of those helpers (the compiler names them).

## Files

| File | Responsibility |
|---|---|
| `architecture.rs` | Layer boundaries, `AGENTS.md` presence, doc comments |
| `database.rs` | Schema, row-level security, seed, and the constraints the services rely on (skips without `TEST_DATABASE_URL`) |
| `recipes.rs` | The builder against a database: drafts, revisions, build outcome, instantiate, capacity |
| `policy_decider.rs` | The egress check end to end: allow/deny/audit, windows, the compiled hash, the deployment comparison, the node token, and the scoped lookup under the application role (skips without `TEST_DATABASE_URL`) |
| `metrics.rs` | Sample ingest, the hour/day/week reads, rollup idempotency, partition retention, the process table and the inventory snapshot against a database |
| `telemetry_ingest.rs` | The proxy batch against a database: node token fail-closed, network/dns rows, redacted bodies, MCP calls, the batch cap, tenant scope (`feat/proxy-telemetry`) |
| `traffic.rs` | The traffic log: rows, aggregates, the map, and the scoped reads |
| `node_ops.rs` | Node summary and drain against a database (B07; written, run by the coordinator) |
| `permissions.rs` | Roles per scope: a viewer added to the data center is listed as direct, the last admin cannot be removed (B12) |
| `bulk.rs` | A bulk stop runs as one job with one result per target (B03; needs PostgreSQL) |
| `agent_auth.rs` | Agent sign-in modes against a database: the default from the old `login` choice, a switch stored as waiting, a restart that keeps the mode, and the sign-in rules appearing and disappearing with the mode |
| `quick.rs` | Recording off stores no telemetry event and recording full stores it again (B11) |
| `model_grants.rs` | Model grants against a database: an environment grant reaching a sandbox, a member refused the environment scope, the narrower sandbox grant winning, and the accounting router payload |
| `moves_preflight.rs` | The move preflight against a database: the sandbox's own node, an unknown node, and what the answer says it carries |
| `mounts.rs` | B09: mount, change, remove and wrong-node refusal (needs TEST_DATABASE_URL; db `nestbox_cc_b09`) |
| `model_bulk.rs` | The bulk model install against a database (`nestbox_cc_b04`): two nodes give two targets and a job whose total is the non-skipped count; a request without exactly one selector is refused |
| `model_clusters.rs` | Cluster create with two installs, provider profile with the cluster endpoint, proxy view, wrong-model refusal, delete removes the profile (DB `nestbox_cc_b05`; compiled only here) |
