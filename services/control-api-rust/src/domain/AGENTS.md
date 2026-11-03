# domain/ — pure types

## Purpose

The vocabulary of the system: sandboxes, policies, gateways, telemetry events,
and the validation that belongs to them. A type here is what a business rule is
written against.

This layer exists so that a rule can be expressed without a database and tested
without a network. It is the cheapest layer to change and the one everything
else depends on.

## Dependencies

- May use: the standard library, `serde`, `chrono`, `uuid`, `utoipa`
- Must not use: `sqlx`, `axum`, `reqwest`, `tokio`, `async fn`

The absence of `async` is deliberate. A domain type that awaits something has
acquired an execution model, and the reason this layer is testable in
microseconds is that it has none.

## Security rules

1. Validation lives here, next to the type. A `SandboxName` that validates in
   the API layer can be constructed invalid from the service layer.
2. No secret values. A type may hold a `secret_ref` (a locator); it may never
   hold a key, token, or password.
3. String length limits are enforced in the constructor, so an unbounded string
   cannot reach a query or a log.

## Conventions

- A constructor validates. `SandboxName::new(...)` returns `Result`, and the
  inner field is private.
- Enums, not stringly-typed states. A status is a Rust enum with an explicit
  database representation.
- No invariant is left to the caller when the type can carry it.

## Files

| File | Responsibility |
|---|---|
| `sandbox.rs` | Sandbox, status, health, spec, version |
| `gateway.rs` | Gateway, status, settings drift |
| `supervisor.rs` | Supervisor and its reported state |
| `policy.rs` | The four-section document and its validation |
| `network_rule.rs` | A network rule and its enforcement mode |
| `telemetry.rs` | Network, DNS, and inference events |
| `job.rs` | Job, status, progress |
| `model.rs` | Model, hardware profile, fit classification, resource bars |
| `model_install.rs` | One model on one node; reservation rule and the free budget the fit runs against |
| `node.rs` | Node, kind, status, budget (total / reserved / free) |
| `pack.rs` | Baseline, agent pack, pack defaults, tool and package-manager resolution |
| `builder.rs` | Recipe, agent spec, home mode, persistence, resource limits, the supervisor script (one loop per agent, PID file in `/tmp/nestbox-agents`) |
| `persist.rs` | Persistent root, mount, mount spec, path derivation |
| `pagination.rs` | Cursor, filter, sort, facet types |
| `auth.rs` | User, role, permission |
| `metrics.rs` | `docker stats` sample, node ingest batch, one-minute bucket, window with its point budget, process table, inventory probe |
| `access.rs` | SSH key record (locator, never material), published port, inventory, session |
| `egress.rs` | The pure `decide` function (rules, then a covering window, then the allow-all answer, each with `would_deny`), compiled rule set, traffic event, map edge |
| `geography.rs` | Reading a country, an AS number, and an owner out of an RDAP record; pure |
| `pack_access.rs` | A pack's words for its ports and SSH line, joined with what the runner reports is open; pure |
| `stack_topology.rs` | The stack diagram in nine layers, arranged from facts; the status vocabulary |
| `mcp.rs` | MCP server definition, sandbox attachment, tool call, JSON-RPC recognition |
| `media.rs` | Media kind, target, the sandbox-over-environment rule, the allow-list refusal |
| `agent_config.rs` | The startup files an agent gets (`settings.json`, `opencode.json`), the gateway environment, and the model-router base URLs; a granted model list beats a single profile model |

## Extending this module

Add the type with a validating constructor, then the error variants it can
produce, then a unit test that constructs it valid *and* invalid. If the type
needs a new state, add the enum variant and the database representation in the
same change, and update the `CHECK` constraint in the migration.

`tenancy.rs` holds the pure rules of the tenant tree (slugs, organisation kinds, invitation roles) and
`hosting.rs` the pure rules of rented machines (budget, lifetime, when to reap).

## Workbox control (ADR 0052-0057)

`agent_auth.rs` holds the agent kinds, sign-in modes and restart policies; `presence.rs` one open
session; `model_grant.rs` a grant, the inheritance rule (narrower scope wins, exactly one default, the
list reads by name); `node_tunnel.rs` a node's tunnel and the
"connect by address" request. `egress.rs` decides with `would_deny` and the `off_allow_all` answer.
`login.rs` turns the in-sandbox probe into a login state and names, per agent, whether an own login
exists (`Probe::accounts`).
session; `model_grant.rs` a grant and the inheritance rule; `node_tunnel.rs` a node's tunnel, the
`status.tsv` dump parser and the "connect by address" request. `egress.rs` decides with `would_deny`
and the `off_allow_all` answer.
`openshell_policy.rs` compiles rules into an OpenShell policy; `allow_hosts` adds the destinations a
suspended sandbox may still reach, in audit mode, because OpenShell 0.1.6 has no host wildcard.
| `privacy.rs` | Detectors, actions, modes, the settings matrix, and which requests are checked |
| `privacy_json.rs` | String values of a JSON document by byte span, so a rewrite touches one value and nothing else |
| `chat_turn.rs` | What one model call says: request, answer, tool calls, tokens, session, provider, agent |
| `privacy_exception.rs` | A finding allowed on purpose, and the specificity rule that picks it |

Control Center (Wave 0; contracts C3-C13 of the plan):

| File | Responsibility |
|---|---|
| `inventory.rs` | `TreeView`, `TreeNode`, `TreeCounts`, `PlacementRow` and the pure `build_tree` (B01 fills it) |
| `model_cluster.rs` | `ModelCluster`, `ClusterMember`, `ClusterStrategy` (`as_str`/`parse`) |
| `search.rs` | `SearchKind`, `SearchHit`, `SearchParams`, `score`, `score_with_id`, `like_pattern`, `console_path` |
| `alerts.rs` | `AlertView` and `alert_id` (stable digest of sandbox, host, port) |
| `mounts.rs` | `MountSpec`, `EffectiveMounts` and `validate_mount_path` |
| `permissions.rs` | `ScopeType` and `PermissionRow` |

`job.rs` gained `ModelBulkInstall`, `NodeDrain` and `NodeQuickDeploy`; `telemetry.rs` gained `LiveEvent::Alert`.
