---
title: Getting started
slug: getting-started
tags: start, shell, tree, roles, sandbox
anchors: [the-screen, roles, create-your-first-sandbox, where-to-go-next]
---

# Getting started

Nestbox runs AI agents in isolated sandboxes and shows you what each sandbox does on the network. You decide what it may reach. This page explains the screen and gets you to a first working sandbox.

## The screen

The console has four parts that never move.

- **Sidebar (left).** The inventory tree. It has three views: *Infrastructure* (organisation, data center, site, node, sandbox), *Workloads* (organisation, data center, person, sandbox) and *Models* (model, install on node). Type in the filter box to narrow the tree. Drag the edge to resize it, or collapse it to icons.
- **Topbar.** The context picker (which organisation and data center you are working in), the search box (press `Ctrl+K`, or `Cmd+K` on a Mac), the bell with open alerts, the help button and your user menu.
- **Content.** Every page starts with a breadcrumb and a title with a status word. The action bar under the title stays visible while you scroll.
- **Dock (bottom).** Three tabs: *Tasks* (jobs with progress), *Live events* and *Alerts* (refused connections waiting for your answer). It is collapsed to one bar by default; press `d` to open it.

## Roles

What you see depends on your role.

| Role | What you can do |
|---|---|
| viewer | Look. Nothing can be changed. |
| member | Work in the sandboxes you own or were given: shell, desktop, your own rules, your own traffic windows, logs, move. Your home is **My sandboxes**. |
| operator | Run all sandboxes of the data center and edit policies. |
| admin | Everything in the data center, including people, models, nodes and providers. |

Hidden buttons are only a convenience. The server refuses anything your role does not allow. See [Customers, data centers, sites](/docs/tenant-tree) for how roles reach down the tree.

## Create your first sandbox

1. Make sure the context picker shows the data center you want.
2. Open **Sandboxes** in the sidebar menu and choose **New sandbox**.
3. Pick a tool image (for example Claude Code) and give the sandbox a name. Names are unique on the whole installation.
4. Choose a policy. If you are unsure, keep the default; you can change it later without losing work.
5. Press **Create the sandbox** (or **Create from recipe** when you started from a recipe). The sandbox appears in the tree and in the dock's *Tasks* tab while it starts.
6. When its status reads *running*, press **Console** to open a shell.

If a node is missing, an administrator adds one first: see [Nodes](/docs/nodes).

## Where to go next

- Run a model on your own hardware: [Deploy one model](/docs/deploy-a-model).
- Decide what a sandbox may reach: [Policies and the wizard](/docs/policies).
- Learn the shortcuts: [Keyboard shortcuts](/docs/shortcuts).
- Unknown word? See the [Glossary](/docs/glossary).
