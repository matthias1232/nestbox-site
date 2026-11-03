# Organisations, data centers, people

Nestbox serves several customers from one installation. This is how the pieces fit
(decisions: ADR 0044, 0045, 0046).

```
Platform (you, the provider)
 +- Partner (optional reseller)
     +- Customer (organisation)
         +- People       roles, down to one sandbox
         +- Data center  where one team runs its sandboxes
             +- Site     home | cloud | edge: a place that has nodes
                 +- Node a machine, joined over the overlay
                     +- Sandbox
```

* A **data center** is the isolation boundary: every row of data carries it, and PostgreSQL itself
  refuses rows of another one (row-level security), on top of the checks in the API.
* A **provider's administrator** reaches the customers below it; a customer's administrator reaches
  only their own data centers. The switcher at the top of the menu lists exactly what you can open.
* **Nodes belong to a customer.** A customer brings their own machines (or rents them under
  *Rented machines*); the provider's nodes are not shared with customers.
* **A sandbox name is unique on the whole installation**, because it is also the container's name.

## Roles

| Role | Does |
|---|---|
| admin | everything in the data center: users, providers and keys, nodes, global policy |
| operator | runs sandboxes and edits policies |
| **member** | works in the sandboxes they own or were given: shell, packages, their own rules and traffic windows, logs, backup, move. Nothing else exists for them |
| viewer | read only |

Give one person one sandbox: *Users -> Share a sandbox*, or invite someone straight into it:
*Users -> Invitations -> Invite someone -> choose the sandbox*. The person signs in (Google, GitHub,
a password) and sees that sandbox and nothing else.

## Becoming a customer (hosted mode)

With `NESTBOX_MODE=saas` and an identity provider configured, the first sign-in creates an
organisation, a first data center ("Main") and a first site, and makes the person its administrator.
They add their first node under *Nodes -> Add a node*.

A provider's administrator can also create customers: *Data centers & sites -> Add a customer*.
