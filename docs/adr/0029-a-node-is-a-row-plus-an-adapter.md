# ADR 0029 — A node is a row plus an adapter; multi-node is not a rename

Status: Superseded by ADR 0047
Date: 2026-10-28

## Context

The control store has carried more than one node since ADR 0018: `nodes` is a
table, `sandboxes.node_id` and `model_installs.node_id` are NOT NULL, and the
budget is computed per node. The runtime, however, is one `RuntimeAdapter`
instance configured from the environment, pointed at one runner. That is why
`GET /nodes` can list one row and the product is still honest: the row names
the machine the adapter actually talks to.

Two questions keep coming back. Can an operator register a second machine
today? And what would it take? This ADR answers both, so the answer stops
being rediscovered in review.

## Decision

**A node row without an adapter is inert, and the API says so.** There is no
create or delete endpoint for nodes. Registering a second row would produce a
node that lists a budget nobody measured and accepts sandboxes no runner can
start. Until an adapter exists for that machine, the row would be a lie in the
topology, so the API refuses to make one.

**The adapter stays per-process, not per-row.** `RuntimeAdapter` is chosen at
startup from configuration (`NESTBOX_RUNNER_URL`, the node token, the model
image allowlist). One control API serves one runner. That keeps the failure
modes simple: a request either reaches the machine the deployment names or it
fails, and there is no dispatch table to get wrong.

**Multi-node is a dispatch problem, not a schema problem.** The schema is
already node-scoped. What is missing is:

1. A registry of adapters keyed by `node_id`, built at startup from
   configuration, with a health check per adapter. The trait already returns
   `RuntimeError`; an unreachable node becomes a typed error instead of a
   panic.
2. Per-node credentials. The node token is a locator per row, and the secret
   directory is already per node; the adapter registry would resolve the
   locator when it builds the adapter, not per request.
3. Node-aware dispatch in every service that currently calls `ctx.runtime`.
   Reservation is already serialised per node in PostgreSQL, so two installs
   on two nodes stay independent without new locking.
4. Reachability from the control API to every runner, and the reverse for
   events. Today control API and runner share a Compose network; a second
   machine needs an address and a token, not a new protocol.
5. Image distribution per node. Digests are pinned in the database, so every
   node pulls the same bytes; nothing needs to be copied between machines.
6. A placement decision: which node a sandbox or install goes to. Today the
   caller names a node or gets the only one. A scheduler that reads budgets
   and picks a node is a product decision, not a schema change, and it should
   not be improvised in the browser.

## Alternatives rejected

**A node URL on the row, read per request.** Tempting because it needs no
startup registry, but it turns a configuration mistake into a request-time
error on one node while the others keep working, and it moves credential
resolution into the hot path. The token locator belongs to adapter
construction.

**Kubernetes as the multi-node substrate.** It would solve placement and image
distribution at once, but it also replaces the Docker socket contract the
runner owns and the bind-mount persistence model in ADR 0021. That is a
different product, not a step.

**Letting the browser group by a guessed node.** Rejected in the console work:
topology groups by `node_name` from the API, because a second grouping rule in
the browser is a second source of truth. The API returns `node_name` on every
sandbox row for that reason.

## Invalidation conditions

This decision is revisited when one of these becomes true:

- An appliance needs two machines to hold the workloads it is sold for.
- A single runner cannot keep up with the sandbox count on one machine.
- A customer requires placement across failure domains.
- The OpenShell adapter lands and brings its own multi-node control path.

Until then, "one node, one adapter" is the design, and a second node row is
refused rather than faked.
