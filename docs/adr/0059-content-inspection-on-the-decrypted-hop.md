# 0059 — Content is inspected on the decrypted hop, in the broker, not in a filter

## Context

The privacy layer could evaluate content and record what it found, and two
callers used it: the accounting proxy (when an agent's SDK points at
`/s/<sandbox>/…`) and the test button on the settings screen. Real egress did
not. A sandbox in `transparent` or `hybrid` mode had its TLS terminated, its
bodies captured for the traffic log, and nothing checked: the DLP screens would
have shown "no findings" for a sandbox that had just sent a customer's IBAN to a
model provider.

That gap was invisible in the UI, which is the worst kind: the page said
"inspected" because the mode is called that.

## Decision

The check runs in `egress-mitm`, the Go broker that re-encrypts the decrypted
hop (`services/egress-proxy/mitm/dlp.go`). It holds the plaintext body, can
replace it, and can answer `403` before a byte leaves.

* **Which requests.** `POST`/`PUT`/`PATCH` with a JSON content type and a body up
  to 4 MiB — the same limit Envoy buffers. A larger body is forwarded
  uninspected, not truncated: half a JSON document is not a smaller document.
  Every other body (uploads, form posts, binaries) is not read, because reading
  it would mean rewriting it.
* **Who it is.** Envoy copies `x-nestbox-sandbox` and `x-nestbox-egress-mode` out
  of the policy answer into the upstream request
  (`authorization_response.allowed_upstream_headers`). The broker sends the id to
  `POST /api/v1/internal/privacy/inspect`; the control API resolves the tenant
  from the id and writes the incident. The client cannot forge either header: the
  Lua filter removes every `x-nestbox-*` header from the sandbox's request first.
* **Authentication.** The same node id and token the tunnel check already uses.
  The endpoint also accepts the accounting proxy's ingest token, which is how the
  older caller reaches it. Two appliance processes, one endpoint, no second
  credential to rotate.
* **Fail modes.** An unanswered check is a `403` in `strict` and `review`, and a
  forward in `observe` — the only mode that promises to look without acting, and
  therefore the only one that may keep its promise while the control API is
  unreachable.
* **Direction.** Only requests are inspected. An answer that streams token by
  token cannot be rewritten without breaking the stream, so the outbound
  direction is recorded only where a caller inspects it explicitly. Both the tab
  and the log say this rather than implying a symmetry that does not exist.

## Why not the Lua filter

Envoy's Lua filter can read a buffered body and is already in the chain. It
could call the control API. It cannot rewrite a request body: there is no
supported write path for it, and a filter that pattern-matches JSON and splices
bytes without a parser is a filter that corrupts requests it does not
understand. The broker is a real HTTP hop with an `io.Reader`; rewriting there is
a two-line change, and the tests are Go tests.

## What this does not cover

* **`forward_proxy` and `direct`.** A tunnel that keeps TLS intact cannot be
  read, and `direct` does not go through the proxy at all. The sandbox tab says
  so; it does not show an empty dashboard that reads as "clean".
* **MCP tool arguments** are inspected because they travel as JSON over the same
  hop, and the row is marked `source: mcp`. Nothing else about the MCP protocol
  is parsed here.
* **A body over 4 MiB** is not inspected. Chat requests that large exist; the
  limit is stated in the UI instead of being silently exceeded.

## Consequences

Every inspected request costs one loopback HTTP call (`1.5 s` timeout). A finding
is written inside the same transaction as the policy read, so the row describes
the policy that produced it. The incident rows now carry direction, the requested
and the effective action, the mode, the substitute, the source, and the request's
address — which is what makes "blocked, changed, passed and only logged" a
sentence a reader can check.
