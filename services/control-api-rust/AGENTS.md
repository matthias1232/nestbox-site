# control-api-rust — the control plane

The Rust/Axum service: the API, the domain rules, and the PostgreSQL store. It is
the only writer to the control database, and the only component that talks to a
node's runner.

## Where things are

| Path | What it holds |
|---|---|
| `src/api/` | HTTP handlers: parse, delegate, format. No rules, no SQL |
| `src/services/` | Business rules, one file per subject; the only place a rule lives |
| `src/store/postgres/` | SQL and row mapping; the only place SQL lives |
| `src/domain/` | Types and the rules that are pure arithmetic (fit, budget, validation) |
| `src/runtime/` | The boundary to the runner: Docker today, OpenShell later |
| `src/openapi.rs` | The published contract (ADR 0015): every route and operation |
| `migrations/` | The schema, one numbered file per decision; never edited after merge |
| `tests/database.rs` | The schema, RLS, seed, and concurrency checks that need a database |
| `tests/architecture.rs` | The boundaries, enforced: layers, no `unwrap`, folder docs |

## Build and test

```bash
export CARGO_HOME=/tmp/opencode/cargo-home   # on the appliance host
cargo check --all-targets                    # the binary is a second crate root
cargo test --lib --test architecture
cargo test --test database -- --test-threads=1   # needs TEST_DATABASE_URL
```

`cargo fmt` and `cargo clippy` need the rustup components; the deployment image
runs `cargo build --release --locked`, so a binary-only compile error is a
deployment failure even when the library tests are green.

## The rules that are enforced, not agreed

1. A layer only calls the layer below it: `api → services → store → postgres`.
   `tests/architecture.rs` fails a violation.
2. No `unwrap`/`expect` outside tests.
3. Every query that touches tenant data filters by `environment_id`, and
   row-level security refuses the rest (ADR 0004). The policy on `sandboxes` is
   also what makes a member's view narrow: a member and a viewer see the
   sandboxes they own or were granted, an admin, an operator and the control
   plane see every one, and an unowned ("shared") row is for the last three
   (migration 0090). A list therefore needs no visibility filter of its own —
   and must not add one that disagrees with the policy.
4. Every public function has a doc comment saying what it does and why.
5. A route registered in `api/mod.rs` has an entry in `openapi.rs`; a stub
   answers `501 server.not_implemented` until its handler exists.
6. Every folder under `src/` has an `AGENTS.md`; a missing one fails the test.

## Wiring a new feature

1. Migration first (numbered, additive), then `store/`, then `domain/` if a type
   is missing, then `services/`, then `api/`, then the OpenAPI entry.
2. A read path is one transaction; a network call (runner, provider) happens
   outside it.
3. A state that can hang (`building`, `starting`, `running`) needs a settlement
   path for the restart that interrupts it.
