# Nestbox — Repository Guide

This file is the entry point for humans and AI agents working in this repository.
Read it first; it tells you where things live and how to change them safely.

## What Nestbox is

Nestbox is an API-first control plane for governed AI-agent sandboxes.
It wraps NVIDIA OpenShell, adds a PostgreSQL control store, a policy and
telemetry layer, and a web console, so that operators can run isolated
workloads and see, allow, or block what those workloads do.

## The one rule that matters

**PostgreSQL is the single control store.** There is no `state.json`, no
in-memory store, and no import path. If you need state, it belongs in a table.

## Repository map

| Path | What it holds |
|---|---|
| `services/control-api-rust/` | The Rust control API. See its `AGENTS.md`. |
| `services/web-console-solid/` | The SolidJS console. See its `AGENTS.md`. |
| `services/sandbox-runner/` | Python runtime adapter speaking to Docker. |
| `services/gateway/` | The OpenShell gateway image and config. |
| `deploy/` | Compose files, scripts, secrets layout. |
| `docs/adr/` | Architecture decision records. Read these before changing a design. |
| `docs/brain/` | Project knowledge for agents, in prose. |
| `docs/architecture/` | Published architecture notes and data flow; `privacy-module.md` describes the content check end to end. |
| `graphify-out/` | Generated knowledge graph. Do not edit by hand. |
| `index.html`, `docs/` | The public website. It is the product specification. |

## Where do I change what?

| I want to change | Go to |
|---|---|
| An HTTP endpoint | `services/control-api-rust/src/api/` |
| A business rule | `services/control-api-rust/src/services/` |
| A SQL query | `services/control-api-rust/src/store/postgres/` |
| A data type or validation | `services/control-api-rust/src/domain/` |
| The database schema | `services/control-api-rust/migrations/` |
| A container lifecycle call | `services/control-api-rust/src/runtime/` |
| Auth, CSRF, redaction | `services/control-api-rust/src/security/` |
| The live event stream | `services/control-api-rust/src/events/` |
| A screen or feature in the console | `services/web-console-solid/src/features/` |
| A reusable UI component | `services/web-console-solid/src/components/` |
| How the console talks to the API | `services/web-console-solid/src/api/` |
| Why a design decision was made | `docs/adr/` |

## Build and test

```bash
# Backend
cd services/control-api-rust
cargo fmt --check
cargo clippy --all-targets -- -D warnings
# `--all-targets` is not optional: src/main.rs is a separate crate root, so a
# `crate::` path that only makes sense in the library compiles under
# `cargo test --lib` and fails the release build the image needs.
cargo check --all-targets
cargo test

# Frontend
cd services/web-console-solid
npm ci
npm run build

# Deploy (on the appliance host)
cd deploy
docker compose up -d --build
```

Integration tests need a PostgreSQL instance. `deploy/docker-compose.test.yml`
provides one; `DATABASE_URL` points at it.

## Conventions you must follow

1. **Documentation is written during implementation, not afterwards.**
   Every folder under `src/` has an `AGENTS.md`. A folder without one fails CI.
2. **One file, one responsibility.** The filename says what the file does.
3. **English everywhere in code and docs.** Code, comments, doc comments,
   commit messages, `AGENTS.md`. Conversation with the maintainer may be German.
4. **Layer boundaries are enforced by tests**, not by good intentions. See
   `services/control-api-rust/src/AGENTS.md`.
5. **No `unwrap()` or `expect()` outside tests.**
6. **Every public function has a doc comment** stating what it does and why.
7. **Secrets never enter the database.** Store a locator, never a value.
8. **Every non-obvious decision goes into an ADR.** The reasoning is more
   valuable than the code, because the code already says what it does.

## Security rules that apply everywhere

- All SQL uses bound parameters. Never build a query string.
- Every query that touches tenant data filters by `environment_id`.
- Permission checks live in the service layer, not only in the router.
- Secrets are redacted from logs, errors, and responses by a central filter.
- Containers run without privileges: no `privileged`, no host network,
  no host mounts, capabilities dropped.
- Images are pinned by digest; `latest` is rejected.

## Working with an agent

If you are an AI agent:

1. Read this file, then the `AGENTS.md` of the folder you are about to change.
2. Read the relevant ADRs before changing a design decision.
3. If the change is structural, write the ADR first.
4. Update the `AGENTS.md` of every folder you change, in the same commit.
5. Run `cargo test` and `npm run build` before claiming the work is done.
6. Never delete a file you did not create without checking `git log` for its
   purpose. It may be deliberate.

## Known limits of the current phase

See `docs/architecture/scope.md` for what M1 deliberately does not do yet, and
`docs/architecture/phase1-review.md` for what is proven on the appliance, what
is partial, and what is not done — with the evidence for each.
