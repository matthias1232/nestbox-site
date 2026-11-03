---
title: Mounts and folders
slug: mounts
tags: mounts, folders, persistent, read-only, read-write, recreate
anchors: [what-a-root-is, see-what-a-sandbox-can-touch, mount-another-folder, read-only-or-read-write, apply-the-change]
---

# Mounts and folders

A sandbox's own disk is discarded when it is recreated. A *persistent root* is a folder that survives: it lives on one node and can be mounted into sandboxes on that node (ADR 0049).

## What a root is

Each root has a short name (its slug), a kind (project or personal), an owner and exactly one node where its files live. A sandbox can mount only roots of its own node. To use a root elsewhere, move the sandbox (see [Moving a sandbox](/docs/moves)); the roots travel with it.

## See what a sandbox can touch

Open the sandbox in the Admin Center and choose the *Mounts* tab. The card **What this sandbox can touch** shows three lists.

- Mounted roots, each with its path inside the sandbox and a chip *read-only* or *read-write*.
- **Read-only paths** and **Read-write paths** from the sandbox's filesystem policy. If the policy has none, the runner's defaults are shown and labelled so.
- Roots you could still mount.

## Mount another folder

1. Press **Mount another**.
2. Choose a root from the list, or press **New folder…** to create one on the sandbox's node.
3. Type the path inside the sandbox. It must be absolute, must not contain `..`, and must be unique among the mounts.
4. Choose read-only or read-write.

## Read-only or read-write

Click the chip on a mounted root to flip it. Use read-only for reference material you do not want an agent to change. The chip always carries its word, never only a colour. Filesystem policy paths are separate: they are the rules OpenShell enforces, while a mount decides whether the folder exists at all. A path that the policy keeps read-only stays read-only even if the mount is read-write.

## Apply the change

Changes are staged first. The button tells you what will happen:

- **Apply now** unmounts and mounts the differences and recreates the sandbox. Running processes end.
- Apply later: the change is saved and the sandbox shows *pending recreate* until it is recreated.

A root that is mounted shows state *mounted*; one waiting for a recreate shows *pending recreate*. Policy changes to the filesystem section follow the same rule (see [Policies and the wizard](/docs/policies)).

## Related

- [Your sandbox](/docs/workbox)
- [Glossary](/docs/glossary)
