---
title: Customers, data centers, sites
slug: tenant-tree
tags: tenants, organisation, data center, site, roles, context
anchors: [the-tree, the-context-picker, who-sees-what, add-a-person]
---

# Customers, data centers, sites

Nestbox can serve several customers from one installation. Everything hangs off one tree (ADR 0044).

![Tenant tree](/illustrations/docs-tenant-tree.webp)

## The tree

- **Organisation.** A customer, or the provider itself. Organisations can be nested: a provider may have partners, and partners have customers.
- **Data center.** Where one team runs its sandboxes. It is the isolation boundary: every row of data carries its data center, and the database itself refuses rows of another one.
- **Site.** A place that has nodes: *home*, *cloud* or *edge*. If you know vCenter, a site is what it calls a cluster: the nodes of one place or provider inside a data center.
- **Node.** A machine that runs sandboxes. See [Nodes](/docs/nodes).
- **Sandbox.** One isolated workload on one node.

A node belongs to one customer. Customers bring their own machines or rent them; the provider's nodes are not shared with customers.

## The context picker

The picker in the topbar lists exactly the organisations and data centers you may open. Choosing one changes everything the console shows: the tree, the lists, the models and the live events. If a page looks empty, check the picker first.

1. Click the context picker in the topbar.
2. Choose an organisation, then a data center.
3. The tree and the page reload for that data center.

## Who sees what

- A **provider's administrator** reaches the customers below it. A customer's administrator reaches only their own data centers.
- An **administrator** of a data center sees all of it, including budgets and node details.
- An **operator** sees all sandboxes of the data center and edits policies.
- A **member** sees their data center, site and node by name only, and their own sandboxes. Nothing else exists for them.
- A **viewer** sees what an operator sees, read only.

A role can also be given for a single sandbox, so one person can be a member of exactly one box.

## Add a person

1. Open the data center page and choose the *Permissions* tab.
2. Press **Add person**, or **Invite by link** for someone who has no account yet.
3. Choose the role and, for a single sandbox, the sandbox.
4. The person signs in (Google, GitHub or a password) and sees what you gave them.

The tab shows where each right comes from: directly, through the organisation, or through a grant on one sandbox.

## Related

- [Getting started](/docs/getting-started)
- [Glossary](/docs/glossary)
