---
title: Nodes
slug: nodes
tags: nodes, add node, tunnel, drain, delete, egress lock
anchors: [add-a-node, run-a-script, connect-by-ip, rent-a-machine, tunnel-state, egress-lock, drain-a-node, delete-a-node]
---

# Nodes

A node is a machine that runs sandboxes. It joins the control plane through an encrypted tunnel (ADR 0047), so it needs outbound internet and no open port.

## Add a node

Press **Add node** on the Nodes page, on a site page, or in the quick actions on Home. One wizard covers all three ways. Pick the source, then the placement (data center, site, node name), then optionally a model to deploy when the node comes online, then review and finish. Nothing is written before you finish.

### Run a script

Choose this for any machine you can log in to. The wizard shows one line to paste on the machine as root. The token works once and for 15 minutes. The script installs Docker and WireGuard if needed, creates the node's own key (the private half never leaves the node), joins the tunnel and starts the runner. The node is online within about a minute.

### Connect by IP

Enter the address, the SSH user and either a password or a private key. Nestbox logs in, runs the same join and shows the steps as they happen. The credentials are used for this job only.

### Rent a machine

Choose a provider account (Hetzner Cloud, DigitalOcean, Vultr or Proxmox VE), a machine type (Hetzner shows the hourly price), a lifetime and an idle limit. The machine is created, joined and later deleted when its time is up or it has been empty for the idle limit. It is never deleted while data is on it.

## Tunnel state

The node page shows the tunnel as *up* or *down*, with the time of the last handshake and the bytes moved. *Down* means the control plane cannot reach the runner; the load gauges then read "Runner unreachable". Check that the machine is on and has internet.

## Egress lock

The egress lock decides what the node's own proxy allows its sandboxes to reach even when a policy says yes. *Strict* allows only what policies allow. Keep it on *strict* unless you have a reason.

## Drain a node

Draining empties a node before maintenance.

1. Open the node page and press **Drain…**.
2. Choose a target node; Nestbox moves every sandbox there after a preflight check (see [Moving a sandbox](/docs/moves)). Or switch on **Stop only** to stop the sandboxes where they are.
3. Follow the job in the dock's *Tasks* tab. The result lists what moved, what stopped and what failed.

Model installs are not touched by a drain.

## Delete a node

Press **Delete node** followed by the node's name on the node page. The dialog drains first. A node that still has sandboxes, models or folders cannot be removed until they are moved. A rented node is handed back to the provider once its last snapshot is stored.

## Related

- [Deploy one model](/docs/deploy-a-model)
- [Customers, data centers, sites](/docs/tenant-tree)
