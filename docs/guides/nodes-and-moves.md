# Nodes, images, moving work (ADR 0047, 0048, 0049)

## Add a node

*Nodes -> Add a node*, choose a site and a name. You get one line to paste on the machine, as root:

```bash
curl -fsSL "https://<control plane>/api/v1/join/<token>/script" | sudo bash
```

The token works once and for 15 minutes. The script installs Docker and WireGuard if needed, makes the
node's own key (the private half never leaves it), trades the token for an identity, brings up the
tunnel to the control plane and starts the node's runner and egress proxy. The machine needs outbound
internet and nothing else: no open port. It shows up online within a minute with its CPU, memory and
GPUs measured.

Other ways to get the same result:

* **cloud-init**: `GET /api/v1/join/<token>/cloud-init` is a `#cloud-config` document that runs the
  join. Paste it into the "user data" field of any provider.
* **ISO / seed**: `deploy/scripts/make-node-seed.sh <token-url>` writes a NoCloud seed ISO to attach to
  any Ubuntu cloud image or VM.
* **A provider account**: *Rented machines* creates the machine and joins it for you.

A node keeps itself current: a timer fetches the node package from the control plane every five
minutes and recreates containers when it changed.

## Images

The control plane runs the one image registry. Publish an image once:

```bash
deploy/scripts/publish-node-images.sh nestbox-claude-code:local
```

then *Image registry & sync -> Assign an image*: to every node of the data center, of a site, or to one
node. Nodes pull what they were assigned (the matrix shows `present`, `pulling`, `failed` per node);
a sandbox started on a node that lacks its image pulls it first. Base layers come from the public
registry, so a new cloud node is ready in minutes without uploading gigabytes from home.

## Move work

*Moves between nodes -> choose a sandbox and a node*. What travels: the sandbox's definition (image,
policies, mounts, MCP attachments), its persistent folders, and the agent's home with its login. Steps:
snapshot while it runs, stop, send the last changes, restore on the target, switch over, recreate,
verify. The way back sends only what changed. The old node keeps its files as a cold copy; if a step
fails before the switch-over, the sandbox is started again where it was.

Snapshots are restic snapshots in one encrypted repository per data center on the control plane's
snapshot server. They double as backups: the data survives the machine, so a rented server can be
deleted once its last snapshot is in.

While a sandbox works on a joined node its folders are snapshotted every 15 minutes
(`NESTBOX_SNAPSHOT_MINUTES`). If the machine is lost, tick *The node is gone: recover from the last
snapshot* on the Moves page: the sandbox comes back on another node with the data of the newest snapshot
(the work since then is lost). Old snapshots are cleared with `POST /api/v1/snapshots/prune`
(administrators; the snapshot server itself never deletes).

A node with sandboxes, models or folders cannot be removed or handed back: move them first.

## Rented machines

*Rented machines*: add a **Hetzner Cloud**, **DigitalOcean**, **Vultr** or **Proxmox VE** account (the token is stored encrypted),
pick a machine type (Hetzner shows the hourly price), a lifetime and an idle limit. The machine joins
by itself. It is deleted when its time is up or it has been empty for the idle limit, never while data
is on it; a budget per account caps what running machines may cost per hour.

Proxmox needs a template once: `deploy/scripts/make-proxmox-template.sh` (an Ubuntu cloud image with
the QEMU guest agent). Any other host: use the one-line command or the cloud-init document.
