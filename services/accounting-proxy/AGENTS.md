# accounting-proxy — model traffic that is accounted for and checked before it leaves

## Purpose

The agent points its provider SDK at this service instead of at the provider.
Every request is inspected by the content-privacy layer before it is forwarded,
and every answer becomes one metadata row. That gives the control plane two
facts it otherwise could not have: **what the agent sent** (a digest, a verdict,
a detector) and **what it cost** (provider-reported token counters), without the
control plane ever holding a prompt or a completion.

Three consequences, stated plainly because they are the point:

- **Credentials are forwarded, never stored.** The caller's own `Authorization`
  (or `x-api-key`) header is copied on unchanged. This service cannot log in,
  cannot mint a token, and holds no provider key. Losing this container's
  environment leaks no provider credential.
- **No content is kept.** Not in a row, not in a log, not in an error. A row
  carries a SHA-256 digest of the request body, so a repeat can be recognised
  without the body being readable.
- **Refusal is a first-class answer.** A blocked request gets `403` and is not
  forwarded, rather than being silently rewritten into something else.

## Contract

The client authenticates with **its own** provider token. There is no auth check
here: the upstream is the authority, and a bad token comes back from the
upstream as it would without this proxy.

| Path | Upstream |
|---|---|
| `/v1/messages`, `/v1/complete` | `NESTBOX_ANTHROPIC_UPSTREAM` |
| `/v1/chat/completions`, `/v1/responses`, `/v1/embeddings` | `NESTBOX_OPENAI_UPSTREAM` |
| `/v1beta/models/...` (Gemini), anything else | `NESTBOX_DEFAULT_UPSTREAM` |
| any path, with `X-Nestbox-Upstream: https://…` | that URL, overriding the table |
| any path under `/s/<sandbox>/r/…` | the granted model's `upstream_base` (see below) |

**Identity in the path.** The egress proxy between a sandbox and this service
drops `X-Nestbox-*` headers, so the sandbox rides in the path:
`/s/<sandbox-name>/…` names it, and an optional `/u/<http|https>/<host[:port]>`
right after names the upstream (the gateway the agent was configured for). Both
are stripped before forwarding. A malformed prefix counts as no prefix. The
control API writes these URLs into the agent environment when
`NESTBOX_ACCOUNTING_PROXY_URL` is set (`agent_config::route_through_accounting`).

`GET /healthz` answers `{"status":"ok","service":"accounting-proxy"}` and needs
no token. Every other method on every other path is forwarded.

**The model router.** `/s/<sandbox>/r/<rest>` is the sandbox's model router: the
control plane grants models per sandbox, and the grant named in the request body
decides where the request goes. The proxy asks
`GET /api/v1/internal/accounting/models/<sandbox>` (same `Authorization: Bearer
$NESTBOX_ACCOUNTING_TOKEN`, same header as the record call) and gets
`{"default": "name", "models": [{"model_name", "upstream_base",
"upstream_model", "authorization"}]}`. The answer is cached for 10 s per sandbox
(`routes_for`); a failed refresh keeps the last known grants, so a control API
blip does not stop a working agent. With nothing cached, the grant list is empty
and every model request gets `404 model_not_granted`.

- `GET …/r/v1/models` is answered here, in the OpenAI list shape, with the
  **granted** names (`{"object": "list", "data": [{"id", "object", "owned_by"}]}`).
  It is not forwarded and no row is written.
- Any other request: `model` from the body, else the sandbox default. No grant
  for that name → `404 {"error": {"type": "model_not_granted", "message": "Model
  <name> is not granted to this sandbox."}}`, nothing forwarded.
- `upstream_model` different from the granted name renames `body["model"]` before
  inspection (so the bytes inspected are the bytes forwarded); `Content-Length`
  follows the new body.
- Upstream is `upstream_base + rest`; the dialect for the row still comes from
  the rest path. A grant's `authorization` replaces the caller's `Authorization`
  header when it is not null, otherwise the caller's header is forwarded as
  usual. `X-Nestbox-Upstream` is ignored on this path: the grant decides.
- The routes document and its `authorization` values are never logged.

**Model clusters.** `/c/<cluster_id>/<rest>` forwards to one healthy member of a
cluster. `cluster_view` fetches `GET /api/v1/internal/accounting/clusters/<id>`
(`{"id","strategy","model_key","members":[{"install_id","endpoint","weight","healthy"}]}`),
cached 10 s with a stale fallback; none at all is `503 cluster.unavailable`.
`choose_member` picks by strategy: `round_robin` (weighted, counter per cluster),
`least_loaded` (fewest in-flight here), `failover` (first healthy). A connection
error or timeout before any byte marks the member down for 30 s and the next is
tried once. The answer carries `x-nestbox-cluster-member: <install_id>`; the
accounting row's `detail` holds `{"cluster_id","install_id"}`. Content inspection
runs as on every other path.

The forwarded request carries the client's headers, minus hop-by-hop headers,
`Host`, `Content-Length` (recomputed) and `X-Nestbox-Upstream` (ours, not the
provider's). The path and query string are preserved. The body is the original
bytes, unless inspection rewrote it.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `NESTBOX_ANTHROPIC_UPSTREAM` | `https://api.anthropic.com` | base URL for the Anthropic paths |
| `NESTBOX_OPENAI_UPSTREAM` | `https://api.openai.com` | base URL for the OpenAI paths |
| `NESTBOX_DEFAULT_UPSTREAM` | *(unset)* | base URL for everything else; unset means `502 upstream.unconfigured` |
| `NESTBOX_CONTROL_API` | `http://127.0.0.1:8080` | control API base URL |
| `NESTBOX_ACCOUNTING_TOKEN` | *(empty)* | Bearer token for the two internal endpoints |
| `NESTBOX_SANDBOX` | *(empty)* | sandbox id written into every row and inspection |
| `NESTBOX_AGENT_INSTANCE` | *(empty)* | agent instance id written into every row |
| `NESTBOX_PRIVACY_FAIL_CLOSED` | `0` | `1` = refuse when inspection is unavailable |
| `NESTBOX_ACCOUNTING_PROXY_PORT` | `8099` | listen port |
| `NESTBOX_UPSTREAM_TIMEOUT` | `120` | seconds for the upstream call |
| `NESTBOX_CONTROL_TIMEOUT` | `5` | seconds for one control API call |

All configuration is environment. There is no config file and no state on disk,
so a restart loses nothing and a container can be replaced freely.

## What the control API is asked, and what it answers

All three calls use `Authorization: Bearer $NESTBOX_ACCOUNTING_TOKEN`. That is a
node token for the internal endpoints, not a provider credential.

**Inspect, before forwarding** — `POST /api/v1/internal/privacy/inspect`:

```json
{"sandbox": "...", "direction": "outbound", "content": "<the request text>", "destination": "api.openai.com"}
```

```json
{"permitted": true, "action": "allowed|replaced|blocked", "transformed_content": "...", "findings": [{"detector": "api_keys", "masked_value": "S*****N", "action": "blocked"}]}
```

The text is the concatenation, with `\n`, of the request's prompt-bearing string
fields: `messages[*].content` (string or text blocks), `system`, `instructions`,
`input` (string or items), legacy `prompt`, embeddings `input` strings, and
Gemini `contents[*].parts[*].text`. Anything else in the body is not sent for
inspection.

**Record, after answering** — `POST /api/v1/internal/accounting/record`:

```json
{
  "sandbox": "...", "agent_instance": "...", "provider": "anthropic|openai|gemini|other",
  "model": "claude-sonnet-4", "direction": "outbound", "endpoint": "/v1/messages",
  "status": 200, "latency_ms": 812,
  "usage": {"prompt_tokens": 5, "completion_tokens": 9, "cached_tokens": 2, "total_tokens": 14},
  "content_digest": "<sha256 hex of the bytes that were forwarded>",
  "privacy_action": "allowed|replaced|blocked|not_inspectable|not_inspected",
  "privacy_detector": "api_keys"
}
```

`usage` is `null` for a stream and for a response without counters. The provider
counters are normalised: OpenAI `prompt_tokens`/`completion_tokens`/
`prompt_tokens_details.cached_tokens`, Anthropic `input_tokens`/`output_tokens`/
`cache_read_input_tokens`, and Gemini `promptTokenCount`/`candidatesTokenCount`/
`cachedContentTokenCount` all land in the same four keys.

**Model grants, for the router** — `GET /api/v1/internal/accounting/models/<sandbox>`:

```json
{"default": "fast", "models": [{"model_name": "fast", "upstream_base": "http://10.90.92.30:20128", "upstream_model": "qwen3:1.7b", "authorization": "Bearer ..."}]}
```

`upstream_base` has no trailing `/v1`; `authorization` is a whole header value or
`null`. This document is cached for 10 s per sandbox and is never logged, because
it carries credentials.

## Rules

1. **Credentials are forwarded, not stored.** No provider key is read from the
   environment, and none is added or rewritten. `Authorization` and `x-api-key`
   are copied as they arrived. The single exception is a granted model on the
   router path whose grant carries its own `authorization`: that header is set
   from the grant, and the caller's own `Authorization` is dropped for that
   request. A grant value never enters a log line, a row, or an error.
2. **No content is logged or stored.** The row carries a digest; the inspect
   call carries the text it must judge and nothing else. `log_message` prints the
   path, the status and the duration — never a header, never a body.
3. **Fail closed only when told to.** `NESTBOX_PRIVACY_FAIL_CLOSED=1` turns an
   unreachable control API into `503 privacy.unavailable` and no forward. The
   default is open, recorded as `not_inspected`: a proxy that is down for
   inspection should not silently take every agent down with it, and the row
   says exactly what happened either way.
4. **`not_inspectable` is not a pass.** A binary body, or JSON with no
   recognisable text, is forwarded unchanged and labelled so. "We did not look"
   must never be indistinguishable from "we looked and it was clean".
5. **A rewrite is applied where it came from.** `replaced` with exactly one text
   field rewrites that field with `transformed_content`. With several fields,
   each is re-inspected on its own and replaced individually, because one
   combined answer cannot be put back into separate messages. A refusal on any
   single field blocks the whole request.
6. **A `replaced` verdict without a `transformed_content` string is refused**,
   not ignored: forwarding the original would silently overrule the policy.
7. **Accounting never changes the answer.** The row is posted after the client
   has it; a failure prints one line and is dropped. Metadata may be lost, an
   answer may not be corrupted.
8. **The upstream override is validated.** Only `http` and `https` are accepted,
   so `X-Nestbox-Upstream: file:///etc/passwd` is a `400`, not a file read. A
   grant's `upstream_base` is checked the same way (`route_base`).
9. **Streams are relayed, not reassembled.** A `stream: true` request (or an
   `text/event-stream` answer) is copied in chunks with a flush per chunk, on a
   closing connection, and accounted for status and latency only.

## Limits worth knowing

- The body is capped at 32 MiB; larger requests get `413`.
- Only `Content-Length` bodies are read. A chunked *request* body is not
  supported (provider SDKs send a length).
- Latency includes the inspection round trip, the upstream call, and the client
  write. It does not include the accounting post.

## Tests

```bash
cd services/accounting-proxy && python3 -m unittest -v
```

Real servers on ephemeral ports: two provider stand-ins and one control API
stand-in, with the proxy between them. Covered: Anthropic/OpenAI/Gemini routing
and the header override, the `Authorization` and `x-api-key` passthrough, a
blocked request not being forwarded, a replaced body (single and per-message),
the exact usage numbers in a row, the digest, `not_inspectable` for a binary
body, streaming with `usage: null`, and both sides of the fail-closed switch.

The router has its own class: a granted model is renamed to its `upstream_model`
and sent with the grant's own key, the caller's key stays when the grant has
none, the default model is used when the body names none, an ungranted model is
a 404 that is never forwarded, `X-Nestbox-Upstream` cannot override a grant,
`/v1/models` lists the granted names, and a routed request still writes a row
with the sandbox from the path. `routes_for` is tested for its 10 s cache, its
stale-on-error fallback and its empty answer without a control API.

Because a row is posted after the answer, each test names its own row by model,
endpoint or privacy action instead of assuming the first row it sees is its own.

## Open work

- The control API's accounting domain (`services/control-api-rust/src/services/
  accounting_service.rs`) currently spells the tenant key `sandbox_id` and its
  counters `usage.{prompt, completion, cached, reasoning}`, while this proxy
  sends `sandbox` and `usage.{prompt_tokens, completion_tokens, cached_tokens,
  total_tokens}`. The internal router that receives a row is where the two
  spellings meet; until it exists, that translation is open.
- `/v1/responses` streaming events carry their own usage in the final event;
  reading it would mean parsing the stream, which the byte-for-byte rule forbids.
- A queue for accounting rows, so a slow control API cannot slow a request
  thread. Today the post is synchronous after the answer.
- A model request that is routed still runs the normal content inspection
  against the destination host, but a grant that renames the model does not
  rename the `model` field of the row: the row names what the upstream ran, not
  the name the agent asked for.
- No invalidation: a changed grant reaches a running proxy within 10 s. There is
  no push from the control API and no per-request bypass.
