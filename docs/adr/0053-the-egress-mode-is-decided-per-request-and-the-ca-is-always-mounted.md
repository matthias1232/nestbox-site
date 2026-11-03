# ADR 0053 — The egress mode is decided per request, and the CA is always mounted

Status: Accepted
Date: 2026-11-01

## Context

Changing a sandbox's egress mode today rebuilds or recreates the sandbox, because
everything that makes a mode work was decided at container creation: `HTTP_PROXY`
and `HTTPS_PROXY`, the trust variables that point at our CA, and the mount that
puts the CA inside the container. A running container's environment cannot be
edited, so the only lever was a new container.

The proxy side was already further along. ADR 0022 put the decision in one proxy
per node that decides nothing itself, ADR 0034 made it identify the sandbox per
connection, and it already answers per CONNECT with the mode named in the
`x-nestbox-egress-mode` header. So the decision was never the problem — only the
sandbox's frozen view of it.

## Decision

* **Every proxy-capable sandbox is created with the egress plumbing on**: the proxy
  environment, the egress CA at `/run/secrets/egress-ca.crt`, and a CA bundle in
  `$HOME/.nestbox/ca-bundle.crt` that `SSL_CERT_FILE` points at. The mode a sandbox
  starts in does not change what is mounted.
* **The mode is a per-request proxy decision.** `PUT /api/v1/sandboxes/{id}/egress-mode`
  writes the row and answers with the setting; the next request through the proxy is
  decided by the new mode. Switching is live for `transparent`, `hybrid`,
  `forward_proxy` and `off_allow_all`.
* **CONNECT tunnels idle out after 10 s**, so an open connection picks up a new mode
  within seconds instead of living as long as the client keeps it.
* **Only `direct` still rebuilds.** It changes the container's network and the node's
  exit proxy, which the container owns; `needs_recreate` is reported for that case
  and for a running container that has no egress CA but is being switched into a mode
  that inspects TLS.
* **The gateway key stays in the container environment.** It cannot be taken out of a
  running container, so ADR 0055 has the agent launcher strip it per mode. The end
  state that the sealed-credential rule of ADR 0016 points at — the proxy adding the
  key per request — is planned, not built.

## Alternatives rejected

* **xDS or another dynamic Envoy configuration per sandbox.** A whole control plane
  for four modes, when the proxy already decides per CONNECT from a header.
* **Changing the container environment live.** There is no such operation in Docker;
  the only approximations are a recreate or rewriting a running process's
  environment, and both are worse than a per-request decision.
* **Keeping "the CA only in sandboxes that inspect TLS".** The first live switch into
  an inspecting mode would then need exactly the recreate this ADR removes, so the
  mount and the bundle have to be unconditional and present before they are needed.

## Consequences

Every sandbox can inspect TLS the moment it is switched, so a switch is a matter of
seconds and leaves the workload's data untouched. A sandbox in `forward_proxy` or
`off_allow_all` carries a CA it never uses; that is the price of the live switch and
it is a public certificate, not a secret. The egress CA's private key stays in the
proxy and is never mounted into a sandbox. A stopped sandbox sees the new mode when it
starts again.

## Invalidation conditions

When a mode must change something the container owns — its network, its DNS wiring,
its exit proxy — the recreate comes back for that setting only. `direct` is the
current example; a mode that needs a different resolver would be the next.
