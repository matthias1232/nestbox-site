# ADR 0044 — Organisations, data centers and sites are the tenant tree

Status: Accepted
Date: 2026-10-31

## Context

A service provider wants to host many customers, each with their own data centers (places they run sandboxes) and, inside those, their own locations. Today the tree stops at `organizations -> environments`, and a node is a global row (ADR 0029).

## Decision

1. **A data center is an environment.** The table stays; only the console word changes. Every tenant-scoped row already carries `environment_id` and row-level security (ADR 0004), so nothing about isolation is re-done.
2. **Organisations form a tree** (`organizations.parent_id`, `kind` = `platform | partner | customer`). A partner manages its customers; a customer sees only itself.
3. **A site** (`sites`, one per environment) is a place that has nodes: `kind` = `home | cloud | edge`, a provider name, a region and the overlay network it uses. Gateways and nodes point at a site.
4. **A node belongs to one environment and one site** (`nodes.environment_id`, `nodes.site_id`, nullable only for the legacy local node, which belongs to the default environment).
5. **Container names carry the tenant.** A sandbox container is `nb-<environment short id>-<sandbox short id>`, not `nestbox-sandbox-<name>`. Two tenants can then use the same sandbox name. Existing names stay until the sandbox is recreated.

## Alternatives rejected

* **A new `tenants` table above organisations.** Organisations already are the top-level tenant; a second concept would need its own isolation rules.
* **Sites as a label on nodes.** A site owns a network, gateways and a provider; a label cannot.
* **Shared nodes between customers from day one.** Running two customers' sandboxes on one machine needs a stronger isolation layer (gVisor or Kata). Customers bring their own nodes first.

## Invalidation conditions

Revisited when customers must share hardware, or when the tree needs more than three levels.

## Amendment 2026-11-01 - what was built

* **Sandbox names are unique on the whole installation, not namespaced.** A container, an agent
  home and an SSH alias are all named after the sandbox, on every node. Renaming them per tenant
  would touch the runner, the SSH gateway, forwards and the terminal; refusing a name another data
  center already uses (`nb_sandbox_name_taken`, migration 0062) gives the same safety (no collision,
  no ambiguity for the dispatcher) at the price that two customers cannot both have `dev-1`.
  Revisit if that bites.
* **Nodes are usable by their own data center and its organisation** (`nb_node_usable`). The node
  that existed before belongs to the first data center, so the provider's organisation keeps it and
  a new customer starts with none and brings (or rents) its own.
* A provider's administrator reaches the customers below it through the organisation tree
  (`nb_user_environments`), not through membership rows, so removing a customer from the tree
  removes the access with it.
