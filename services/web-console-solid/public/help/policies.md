---
title: Policies and the wizard
slug: policies
tags: policies, wizard, network, hot reload, recreate, drafts
anchors: [the-four-sections, hot-or-recreate, open-the-wizard, review-before-you-apply, drafts-from-real-traffic]
---

# Policies and the wizard

A policy says what a sandbox may do. Nestbox compiles it into the document that OpenShell enforces.

![Policy path](/illustrations/docs-policy-path.webp)

## The four sections

| Section | Controls |
|---|---|
| `network_policies` | Which hosts, ports and programs may be reached. |
| `filesystem_policy` | Which paths are read-only and which are read-write. |
| `landlock` | How strictly the kernel enforces the filesystem rules. |
| `process` | Which user the workload runs as and what it may do. |

## Hot or recreate

Only the network section changes in a running sandbox (ADR 0008). Existing connections open under the old rules are closed at the moment of the change, so a keep-alive or a WebSocket may drop.

The other three sections are fixed when the workload starts. A change is saved, but it applies only after the sandbox is recreated. Removing a path or changing `process` or `landlock` after start is rejected with an error naming the section. The wizard marks each section as *applies now* or *requires recreate*, and lists the sandboxes affected. Recreate is never done silently: you press **Recreate** when you are ready, because it ends the workload's live state.

## Open the wizard

1. Open **Policies** in the sidebar menu and choose a policy, or open a sandbox and press **Open policy wizard**.
2. Move through the steps:
   - *Network*: the allow list. Suggestions come from what the sandbox actually reached; press **Add as allow** next to one.
   - *DNS*.
   - *Filesystem*: the mount editor (see [Mounts and folders](/docs/mounts)).
   - *Process and Landlock*.
   - *Tools and MCP*: what is attached, read only here with links.
   - *Observation*: egress mode and evaluation window (see [Egress modes and observation](/docs/egress-modes)).
   - *Review*.
3. Press Next only when the step has no error. Errors name the exact path, for example `network_policies[0].endpoints[1].port`.

## Review before you apply

The *Review* step shows the compiled OpenShell document, which sections are hot and which need a recreate, a list of rules that cannot be expressed (for example deny rules) and a diff against the document the sandbox has now. Nothing is written before you press the final button. If a sandbox is chosen, the result says whether the change applies now or after a recreate; if you want the recreate at once, press **Apply now**.

## Drafts from real traffic

A blocked connection can become a draft rule instead of an immediate allow.

1. In an alert or the live log, press **Draft rule**. A draft is saved; the sandbox is unchanged.
2. Open **Policies**, choose the *Drafts* tab.
3. Choose **Edit in wizard** to refine a draft, then apply it.

On the workbox the same list can be created in one go from a log: "N drafts created, review them".

## Related

- [Live logs and alerts](/docs/live-logs-and-alerts)
- [Glossary](/docs/glossary)
