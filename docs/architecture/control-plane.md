# Control Plane Architecture

The control plane is four layers with hard boundaries (ADR 0002, enforced by
`tests/architecture.rs`):

| Layer | Answers | Folder |
|---|---|---|
| Domain | What is a sandbox, a rule, an event? | `services/control-api-rust/src/domain/` |
| Store | How is it persisted and read back? | `services/control-api-rust/src/store/` |
| Services | What is allowed, and what happens next? | `services/control-api-rust/src/services/` |
| API | How does it arrive over HTTP? | `services/control-api-rust/src/api/` |

Execution happens in `sandbox-runner`, the only process with Docker socket
access; the API reaches it over an internal HTTP contract. PostgreSQL is the only
control store (ADR 0001): there is no `state.json`, no in-memory store, and no
import path.

## The contract lives in one list

`src/openapi.rs` holds every endpoint as a `Operation` row: method, path,
summary, tag, minimum role, CSRF requirement, and whether it streams (SSE or
WebSocket). The document at `/api/v1/openapi.json` is generated from that list,
and the tests in that file refuse a duplicate path and require every mutation to
declare its CSRF position.

That list, not this page, is authoritative. A route that exists without a row
fails the build; a row without a route is caught in review.

## What the Admin Center adds

Grouped by tab, all under the frozen `/api/v1/` prefix (ADR 0015):

| Area | Endpoints |
|---|---|
| Connect | `GET /sandboxes/{id}/connect` (node address, hostname, ports, SSH), `PUT /nodes/{id}/endpoint` |
| Stacks | `PUT /sandboxes/{id}/stack` (a member joins a lead or leaves; one level deep); a tool or gateway created without a lead joins the node's only agent (ADR 0032) |
| Stack diagram | `GET /sandboxes/{id}/stack-topology` (operator to internet, live state, a log per piece), `GET /system/components`, `GET /system/components/{service}/logs` |
| Dashboard | `GET /sandboxes/{id}/metrics`, `GET /sandboxes/{id}/processes`, `GET /sandboxes/{id}/policies/effective` |
| Traffic | `GET /traffic` (every sandbox of the environment), `GET /traffic/decisions` (denied destinations, grouped), `GET /sandboxes/{id}/traffic`, `GET /sandboxes/{id}/traffic/{event_id}/body`, SSE on `/telemetry/stream` |
| Media | `GET/PUT /media/settings`, `GET/PUT /sandboxes/{id}/media`, `GET /sandboxes/{id}/media/renders`, `GET /media/renders/{id}/asset/{name}`, `POST /internal/media/render` (media token), `GET /internal/media/info`, `GET /internal/media/renders/{id}/asset` |
| MCP | `GET/POST /mcp/servers`, `GET/PUT/DELETE /mcp/servers/{key}`, `GET/POST /sandboxes/{id}/mcp/servers`, `DELETE /sandboxes/{id}/mcp/servers/{key}`, `POST /sandboxes/{id}/mcp/servers/{key}/discover`, `GET /sandboxes/{id}/mcp/calls` |
| Terminal | `GET /sandboxes/{id}/terminal` (WebSocket, `x-stream: ws`) |
| Access | `GET /sandboxes/{id}/ssh-key`, `GET /sandboxes/{id}/ssh-key/download`, `POST /sandboxes/{id}/ssh-key/rotate`, `GET/POST /sandboxes/{id}/ports`, `DELETE /sandboxes/{id}/ports/{forward_id}`, `GET /sandboxes/{id}/sessions` |
| Map | `GET /sandboxes/{id}/map`, `GET /topology` with live edges |
| Inventory | `GET /sandboxes/{id}/inventory`, `POST /sandboxes/{id}/inventory/refresh` |
| Policies | `GET/POST /policies`, `GET /policies/global`, `GET/PUT/DELETE /policies/{id}`, `GET /policies/{id}/diff`, `GET /policies/{id}/versions`, `GET/POST /traffic-windows`, `DELETE /traffic-windows/{id}`, `GET/POST /network-rules`, `DELETE /network-rules/{id}`, `GET/POST /dns-rules`, `DELETE /dns-rules/{id}`, `GET/PUT /sandboxes/{id}/egress-mode`, `GET /sandboxes/{id}/policy`, `PUT/DELETE /sandboxes/{id}/policy`, `GET/POST /policy-templates`, `POST /policy-templates/{id}/instantiate`, `GET /policy-schema` |
| Node ingest | `POST /internal/policy/check`, `POST /internal/telemetry/batch`, `POST /internal/metrics/ingest` |

The three `/internal/` endpoints are authenticated by a **node token**, not a
session: no cookie, no CSRF token. They are enumerated in the CSRF test so a
fourth one cannot appear without a decision.

## Invariants

1. Every query that touches tenant data filters by `environment_id`, and the
   permission check lives in the service layer.
2. No egress route exists without a rule: the proxy asks the control API per
   request, and an unanswered question is a deny (ADR 0022).
3. A state change that matters is recorded in the audit trail, and a change that
   can fail partway is a job rather than a synchronous write.
4. Secrets are locators, never values (ADR 0005). That includes SSH private keys
   (ADR 0024) and provider credentials.
5. A stream says which kind it is: SSE and WebSocket need different gateway
   settings, and the document tells the gateway which one it is looking at.
