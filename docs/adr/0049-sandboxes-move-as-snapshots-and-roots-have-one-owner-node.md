# ADR 0049 — Sandboxes move as snapshots, and a root lives on one node at a time

Status: Accepted
Date: 2026-10-31

## Context

Work should move to a rented GPU machine and back: project files, persistent data and the agent's login (`.agent-state`), without keeping the machine.

## Decision

1. **What moves**: the sandbox definition (image digest, policy, mounts, MCP, packs), its persistent roots, and the agent home with the login. Images are not moved; they are synchronised (ADR 0048).
2. **A restic repository per data center** (S3 or a rest-server on a home node), encrypted and deduplicated, replaces the tar.gz root backups. Backup and migration are the same mechanism.
3. **A lease** in PostgreSQL says which node holds a root. Only the holder runs it, so two live copies of one login cannot exist.
4. **Move job**: live snapshot, short stop, delta snapshot, restore on the target, recreate with the same identity and policy, verify checksums and health, switch the lease, keep the source as a cold copy.
5. **Deleting a node is refused** while a root on it has changes newer than its last snapshot.
6. **Control-plane backup**: pgBackRest to S3; the vault key is backed up separately, because without it every stored key is unreadable.

## Alternatives rejected

* **rsync between nodes.** No history, and the node must be alive for the way back.
* **Shared network filesystem.** Latency makes it unusable across the internet.

## Invalidation conditions

Revisited when roots reach sizes where snapshots are too slow, or when a root must be live on two nodes.

## Amendment 2026-11-01 - what was built

* The **lease is the row**: a persistent root has exactly one `node_id`, and a move changes it in the
  same transaction that changes the sandbox's node, so two nodes never both claim a root.
* The snapshot server is restic's `rest-server`, **append-only**, on the overlay address; each data
  center has its own repository path and its own encryption password (kept in that data center's vault).
  A node can add snapshots but not delete or rewrite one. Pruning old snapshots is therefore an
  administrator task on the control plane and is not automated yet.
* A restore sets the existing folder aside as `<folder>.replaced-<time>` instead of deleting it.
* Proven on the test box: a sandbox with a 3 MB folder moved from the appliance to a joined VM and back;
  the way back sent 0.5 MB (only the change); the file written on the VM was there after the return.
* **Scheduled snapshots** (migration 0072/0075): a folder mounted in a running sandbox on a joined node is
  snapshotted every `NESTBOX_SNAPSHOT_MINUTES` (default 15) and listed under
  `GET /api/v1/persistent-roots/{id}/snapshots`. A node holding folders cannot be deleted or handed back.
* **Pruning**: `POST /api/v1/snapshots/prune` (admin) runs `restic forget --prune` on the control plane's own
  runner against the directory the append-only server writes; no node can delete a snapshot.
* **Recovery**: `POST /api/v1/sandboxes/{id}/recover {node}` brings a sandbox back from the newest scheduled
  snapshots when its node is offline (an online source node is refused: move instead). Proven by powering off a
  rented machine: the sandbox came back on the appliance with the data of the last snapshot.
* Not built: the pgBackRest backup of the control plane (a dump script exists:
  `deploy/scripts/backup-control-plane.sh`), moving a folder shared by several sandboxes, automatic recovery
  (a person decides: recovery loses the work since the last snapshot).
