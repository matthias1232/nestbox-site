# ADR 0065 — A model cluster is installs behind one provider, routed by the proxy

Status: Accepted
Date: 2026-11-02


## Context

One local model on one node is a point of failure and a ceiling. An operator who installs
the same model on four GPU nodes wants a sandbox to use "the model" and get any of the
four. Models are granted per scope and reached through the accounting proxy's router
(ADR 0057); a local model is published on the overlay (ADR 0058). Nothing yet let several
installs share one name.

Not to be confused with a *site*, which the console shows where users say "cluster"
(ADR 0064), or the old gateway `clusters` table.

## Decision

* **A model cluster is a named set of installs of one model** in one data center
  (`model_clusters`, `model_cluster_members`). Every member is an install of the
  cluster's `model_key`; members carry a weight. The strategy is `round_robin`,
  `least_loaded` or `failover`.
* **One provider profile points at the cluster.** Creating a cluster creates one
  `local_openai_compatible` profile named after it, with the proxy's cluster path
  (`/c/<cluster_id>`) as endpoint and no secret. Granting a model works as before;
  deleting the cluster deletes the profile unless sandboxes still use it (409).
* **The proxy picks a member per request.** `/c/<id>/<rest>` behaves like the per-sandbox
  router path but chooses the upstream by strategy: weighted round robin (in-memory
  counter), fewest in-flight requests, or first healthy in member order. On a connection
  error the next member is tried once. The answer names the member in
  `x-nestbox-cluster-member`, and accounting records the install id.
* **Health is what is already known**: install state `installed` and provider status
  `reachable`. The proxy reads the cluster through an internal node-token route and caches
  it for 10 seconds.

## Why the proxy and not the control API

The proxy already sits in the model path, already holds the per-sandbox router, and
already attaches keys the sandbox must never see (ADR 0016, 0057). Putting selection in
the control API would put every model request through the control plane and add a hop
that does not scale with token streams.

## Alternatives rejected

* **A load balancer container per cluster.** Another thing to deploy, monitor and secure
  per cluster; the proxy is already there.
* **Several providers and a client-side choice.** The agent would have to know the
  members, and membership changes would rewrite agent configuration.
* **Sticky sessions.** Inference requests are independent; stickiness only unbalances.

## Consequences

Adding or removing a member is live within the cache window and changes no sandbox. A
cluster is a single failure domain for permissions: granting the model grants the cluster.
Round-robin counters are per proxy process, so balance is approximate with several proxies.

## Not built

Clusters across data centers, health probes beyond provider status, autoscaling members
up or down with load.

## Invalidation conditions

If inference moves to a gateway that does its own upstream selection, the proxy's picker
is removed and the cluster only defines membership.
