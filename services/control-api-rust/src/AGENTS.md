# src/ — the layer model

## Purpose

The control plane, split into layers with hard boundaries. Each layer answers a
different question, and a file belongs to exactly one.

| Layer | Folder | Answers |
|---|---|---|
| Domain | `domain/` | What is a sandbox, a policy rule, an event? |
| Store | `store/` | How is it persisted and read back? |
| Services | `services/` | What is allowed, and what happens next? |
| API | `api/` | How does it arrive over HTTP? |
| Runtime | `runtime/` | How is it executed on a gateway or container? |
| Events | `events/` | How does it reach a browser live? |
| Security | `security/` | Who is asking, and is this safe to output? |

## Dependencies

Allowed, by layer:

```
api       → services, domain, security
services  → domain, store (traits), runtime (traits), events, security
store     → domain
runtime   → domain
events    → domain
security  → domain
domain    → nothing
```

Forbidden, and enforced by `tests/architecture.rs`:

- `domain/` must not contain `sqlx`, `axum`, `reqwest`, or `async fn`
- `store/` must not contain `axum`
- `services/` must not contain `axum::` or `sqlx::`
- `api/` must not contain `sqlx::`
- `runtime/` must not contain `sqlx`

## Security rules

1. Every repository call takes an `environment_id`. It is not optional and it
   is not defaulted, because the default is where leaks come from.
2. Tenant transactions set `SET LOCAL app.environment_id` before any query.
3. No `unwrap()` or `expect()` outside tests. A panic in a request handler takes
   down the task and produces a 500 with no context.
4. Every error that reaches a client goes through `error::ApiError`, which
   produces the published envelope and never internal detail.
5. Secrets are read through `security::redaction` and never stored in the
   database.

## Conventions

- One file, one responsibility. The filename states what the file does.
- Every public item has a doc comment saying what it does and why it exists.
  "Why" is the part a reader cannot recover from the signature.
- Types are named for the domain concept, not the database column.
- A function that needs the word "and" in its name is two functions.

## Extending this module

1. Decide which layer owns the behaviour. If it is a rule, it is `services/`.
   If it is a query, it is `store/`. If it is a status code, it is `api/`.
2. Add the domain type first, in `domain/`. Everything else refers to it.
3. Add the store method and its SQL, in `store/postgres/`.
4. Add the service function, with the permission check inside it.
5. Add the HTTP handler, which parses and delegates only.
6. Write the module `AGENTS.md` for any folder you created.
7. Add the endpoint to the OpenAPI document and to the endpoint list in
   `docs/architecture/control-plane.md`.
8. Run `cargo test`; the architecture test will catch a boundary violation.

## Files

| File | Responsibility |
|---|---|
| `main.rs` | Process start: config, pool, migrate subcommand, serve; before the first request it settles what a restart interrupted (moves, builds, model installs, and the Control Center's own jobs) |
| `lib.rs` | Crate surface: `build`, `app` |
| `config.rs` | Environment to settings, secret files, redaction-safe logging |
| `error.rs` | `ApiError`, the enumerated `ErrorCode` constants, and the published error envelope. A new code is added here rather than spelled inline, and only when no existing code names the condition (P10 added `node.name_taken` and `site.not_found`) |
| `state.rs` | `AppState`: pool, runtime, bus, config |
| `openapi.rs` | The `utoipa` document |
