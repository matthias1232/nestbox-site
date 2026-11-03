# ADR 0042 — Model traffic is routed through the accounting proxy, and the sandbox rides in the path

Status: Accepted
Date: 2026-10-31

## Context

The accounting proxy records tokens and enforces the privacy policy, but only for
traffic that reaches it. Agents pointed straight at their gateway, so nothing was
accounted for. A sandbox also cannot tell the proxy who it is by header: the
egress proxy on the way drops `X-Nestbox-*` headers (the media server hit the same
wall, ADR 0039).

## Decision

1. Routing is **opt-in per deployment**: `NESTBOX_ACCOUNTING_PROXY_URL`. Unset, agents
   keep talking to their gateway directly.
2. When set, the control plane rewrites the agent's `OPENAI_BASE_URL`,
   `OPENAI_API_BASE` and `ANTHROPIC_BASE_URL` to
   `<proxy>/s/<sandbox>/u/<scheme>/<host>…`. The sandbox and the intended upstream
   are in the path; the proxy strips them before forwarding.
3. The client's own credential is forwarded untouched. The proxy never logs in
   for anyone and holds no provider key.
4. The sandbox gets a per-sandbox allow rule for the proxy's host and port, and
   nothing wider.

## Rejected

- **A header for identity.** Dropped by the egress proxy.
- **One proxy container per sandbox.** Fixed cost per sandbox for a lookup the
  path does for free.
- **Routing by default.** Would put a new hop in front of every existing agent
  before the operator chose it.

## Invalidated if

The egress proxy starts preserving a signed identity header, or sandboxes get
their own network namespace address the proxy can read, which would make the path
prefix redundant.
