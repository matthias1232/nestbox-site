# ADR 0047 — Nodes join through an agent and a WireGuard overlay; this replaces ADR 0029

Status: Accepted
Date: 2026-10-31

## Context

ADR 0029 kept one runtime adapter per process and refused to create nodes. The goal is now to attach a machine in minutes, behind NAT, at any cheap host.

## Decision

1. **A node runs the node package**: runner, egress proxy, optionally an OpenShell gateway, and a small **node agent** (join, heartbeat, image sync, snapshots).
2. **Overlay: WireGuard, hub and spoke.** The control plane is the hub; each data center has its own /24. A node needs no open port. The runner API, forwarded ports and data transfer ride on overlay IPs unchanged.
3. **Join by one-time token** (15 minutes, bound to a data center and a site). The node exchanges it for its identity: a WireGuard key and a node token, both stored as vault locators.
4. **Four ways to install**: a one-line script, cloud-init text for a host's user-data field, a bootable ISO, and a provider driver (ADR 0050).
5. **The control API holds an adapter registry keyed by `node_id`** instead of one adapter. Every service call that used `ctx.runtime` dispatches by node. An unreachable node is a typed error. Reservations stay serialised per node in PostgreSQL.
6. **Placement** is explicit at first (the caller names a node); a scheduler that reads budgets comes later.

## Alternatives rejected

* **A node URL read per request.** Moves credential lookup into the hot path.
* **Kubernetes.** Replaces the runner contract and bind-mount persistence (ADR 0021).
* **A public port on every node.** Does not work behind NAT and enlarges the attack surface.

## Invalidation conditions

Revisited when overlay throughput becomes the bottleneck, or when a customer forbids a provider-operated hub.

## Amendment 2026-11-01 - what was built

* The "node agent" is the runner plus a systemd timer (`nestbox-node-sync`) that fetches the node
  package from `GET /api/v1/node/compose` with the node's token, and pulls and recreates when it changed.
* A node reaches the control API itself at `http://10.77.0.1:8000` (forwarded by the hub), **not** the
  console's nginx: the egress proxy asks the API about `CONNECT` requests and nginx refuses that method.
* A joined node's token is kept as `digest://<sha256>` in `nodes.node_token_ref`; the clear value lives
  in the vault for the control plane's calls to the node's runner. Every runner request on a node must
  carry it (`NESTBOX_RUNNER_REQUIRE_TOKEN`).
* Every node's Docker hands out the same private addresses, so the proxy's "who is asking" lookup asks
  only the node that called, never the merged list of all nodes.
* The dispatcher (`runtime/router.rs`) routes calls about one sandbox to its node's runner and merges
  `list`. Calls about nothing in particular (builds, catalogue, model serving, backups) still go to
  the local runner: models and builds on a joined node are not built yet.
* Not built: node-to-node direct links (all traffic goes through the hub), automatic re-placement of a
  sandbox when its node dies.
