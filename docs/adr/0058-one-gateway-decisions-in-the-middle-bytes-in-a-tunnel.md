# ADR 0058 — One gateway: decisions in the middle, bytes in a tunnel

Status: Accepted
Date: 2026-11-01

## Context

ADR 0047 joined nodes through a WireGuard overlay and gave the hub a fixed set of
forwarded ports. Three consequences were not solved by it: a joined node could not
reach the MCP hub, media rendering or the accounting proxy at all; a node's workload
left the internet directly, so the egress decision that ADR 0022 puts in the middle was
bypassed; and the only way to attach a machine was a join command run by hand on that
machine, which needs a shell there before the overlay exists.

The result was a hub that governed one node — the one it ran on — and a set of nodes it
could only watch.

## Decision

* **Every node reaches the hub's services under the names the sandboxes already use.**
  `mcp-hub:8080`, `media-mcp:8080` and `accounting-proxy:8099` on the runtime network,
  and `127.0.0.1:18087` (egress), `18088` (MCP hub), `18089` (media), `18099`
  (accounting) on the host for OpenShell's `host.openshell.internal`. The node package
  installs the forwarders; the hub forwards the same ports on `10.77.0.1`.
* **The hub is the way out.** `10.77.0.1` also NATs overlay traffic to the internet, so
  a sandbox on any node leaves through the hub's exit proxy and the hub's decision.
* **A node runs under an egress lock.** `strict` is a WireGuard full tunnel
  (`AllowedIPs 0.0.0.0/0`) plus a kill switch: nothing leaves the node except through
  the overlay, and every connection is decided in the middle. `exit_local` is the
  previous behaviour — a decision every 3 s from the control API, egress at the node.
  `strict` is the default. The lock is readable at the node
  (`GET /api/v1/node/egress-lock`, node token) and settable by an admin
  (`PUT /api/v1/nodes/{id}/egress-lock`, `EnvironmentManage`).
* **A machine joins by address and credentials.** `POST /api/v1/nodes/connect` takes
  the public IP, the SSH port and credentials, prepares the machine and applies the
  node package. **The password or key is used once and never stored.** The hub dials
  the node's WireGuard port with `PersistentKeepalive = 25`, so a machine behind NAT
  needs no port forward.
* **Tunnel state is a fact, not a guess.** Every 3 s the hub writes `wg show nb-hub
  dump`; the API reports `up`, `stale`, `down` or `unknown` with the handshake age and
  the byte counters, and publishes `NodeTunnelChanged` when the state changes.

## Alternatives rejected

* **A second VPN product beside WireGuard.** Two overlays, two failure modes, and the
  sandboxes would still need to know which one to use.
* **Hub-side TLS over the WireGuard hop.** The node's Envoy already terminates per node
  (ADR 0022, ADR 0034); a second TLS layer would have to reproduce the per-sandbox
  identity that the existing header carries.
* **Requiring a port forward at home.** It rules out exactly the machines the feature
  is for. The hub dials instead.
* **Leaving nodes on their own exit.** It keeps the last mile outside the decision, so
  the traffic log would describe a proxy the traffic did not use.

## Consequences

`strict` is the default and the user accepted what it costs: a download on a joined node
travels through the hub and is slower than the same download at the node. A bulk-download
exception was considered and **deferred** — no exception is built, and if throughput
becomes the problem it becomes a new ADR with its own rule for what may bypass. A node
whose tunnel is down is reported instead of quietly falling back to its local exit; the
kill switch makes the failure visible rather than silent.

## Invalidation conditions

When overlay throughput turns into the bottleneck for model weights, the deferred bulk
exception becomes the answer and needs its own decision about which downloads may leave
locally. When a node can be attached without any credential on the machine at all, the
connect-by-address flow loses its "used once" secret and this ADR is amended.
