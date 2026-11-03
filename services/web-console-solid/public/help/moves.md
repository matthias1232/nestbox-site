---
title: Moving a sandbox
slug: moves
tags: move, node, preflight, rollback, recover, snapshot
anchors: [what-travels, move-a-sandbox, the-steps, roll-back, recover-from-a-lost-node]
---

# Moving a sandbox

A move puts a sandbox on another node together with its files (ADR 0049). Use it to free a machine, to use a bigger one, or to leave a node that is going away.

## What travels

- The sandbox's definition: image, policies, mounts and attached tools.
- Its persistent folders (roots). A root has exactly one home node, and it follows the sandbox.
- The agent's home, including the agent's login.

What does not travel: running processes and anything held only in memory. The sandbox is stopped for the final step.

## Move a sandbox

1. Open the sandbox and press **Move…**. For several at once, tick them in the tree or table and use the bulk bar.
2. Choose the target node. The list marks nodes that do not fit (offline, no image, not enough memory).
3. Read the *preflight* result. It checks that the node is online, has room and has the image. Fix any failure shown, or choose another node.
4. Confirm. The move runs as a job; follow it in the dock's *Tasks* tab.

## The steps

1. Snapshot the files while the sandbox keeps running.
2. Stop the sandbox.
3. Send the last changes (only what changed since the snapshot).
4. Restore on the target node.
5. Switch over.
6. Recreate the sandbox there.
7. Verify that it starts.

Snapshots live in one encrypted repository per data center, so they also act as backups.

## Roll back

If a step fails before the switch-over, the sandbox is started again where it was. Nothing is lost. After the switch-over, the old node keeps its files as a cold copy, and moving back sends only what changed. Read the job's log to see which step failed.

## Recover from a lost node

If a node has died, the sandbox cannot be moved normally. On the Moves page, tick *The node is gone: recover from the last snapshot*, choose the sandbox and a target node. The sandbox comes back with the data of the newest snapshot. Work done since then is lost. While a sandbox works on a joined node its folders are snapshotted every 15 minutes by default.

## Moves and nodes

A node that still holds sandboxes, models or folders cannot be deleted. Use **Drain…** on the node page to move everything off it (see [Nodes](/docs/nodes)).

## Related

- [Mounts and folders](/docs/mounts)
- [Your sandbox](/docs/workbox)
