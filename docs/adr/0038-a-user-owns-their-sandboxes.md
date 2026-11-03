# ADR 0038 — A user owns their sandboxes; an admin sees and sets everything shared

Status: Accepted
Date: 2026-10-29

## Context

Every sandbox of an environment was visible and writable by every member. The roles
(admin, operator, viewer) decided *what kind* of action was allowed, never *on whose*
sandbox. For a product that several people use, and where each should run their own
agents from start to finish, that is not enough, and there was no way to create a user.

## Decision

1. **A sandbox has an owner**, the user who created it (`sandboxes.owner_user_id`).
2. **Row-level security decides who sees it**, in the database, so no endpoint can forget
   the check: an admin sees every sandbox; anyone else sees their own and the shared ones
   (no owner); the control plane itself, acting for nobody, sees all. The acting user and
   role are set per transaction next to the environment (`TenantScope::begin_for`).
3. **What belongs to a sandbox follows it.** Persistent roots and their backups, telemetry
   rows, sandbox-specific network rules and supervisors get the same owner-or-admin policy.
   The live streams drop events of sandboxes the caller may not see. Endpoints that read by
   sandbox id load the sandbox first, so another user's id is a plain not-found.
4. **What is shared is set by admins.** Providers and the keys behind them, gateways, global
   policy, the audit trail and user administration are admin-only; users choose from what the
   admin made available.
5. **Users are created by an admin** (name, email, role, a password of at least 12
   characters). Guards: an admin cannot demote, disable or remove themselves, the last admin
   stays, a role change or a disabled account ends the user's sessions at once, and removing a
   user leaves their sandboxes shared rather than orphaned.
6. Sandboxes that existed before ownership went to the first admin of their organization, so
   adding a user does not expose them.

## Not decided here

* **Several environments as tenants** (an environment per team, onboarding it with a cluster,
  gateway and node) are not built: sandbox names are unique per environment but are container
  and OpenShell names on one host, so they would have to be namespaced first.
* **Policies are shared documents.** An operator may edit a policy that other users' sandboxes
  carry; a per-owner policy scope is the next step.
* Telemetry rows about *no* sandbox (gateway, node) are visible to admins only.

## Amendment 2026-11-03 — a member does not see the shared rows, and a sandbox is owned from the insert on

Point 2 said "anyone else sees their own and the shared ones (no owner)", and a sandbox was owned
by an `UPDATE` after its `INSERT`. Both were wrong once the `member` role existed (ADR 0045): on an
installation whose sandboxes carry no owner — what a deleted creator leaves — a member granted one
sandbox saw every sandbox of the data center in "My sandboxes" (U06-05), because the shared clause
applied to roles that are not supposed to see the shared ones.

* The **member** and **viewer** roles see the sandboxes they own or were granted, and nothing else
  — the definition of the member role in `domain::auth::Role`.
* **Admin**, **operator** and the control plane (no acting user) still see the shared rows, because
  they are the people who adopt or clean up what a removed user left.
* `sandboxes.owner_user_id` is written by the `INSERT` itself, so the row is never briefly unowned:
  a member's own sandbox passes the write policy the moment it exists (migration 0090).

The alternative — filtering in the list query — was rejected: it would let `GET /sandboxes/{id}`
open a sandbox the list hides, and every other reader of `sandboxes` (telemetry, rules, supervisors,
the inventory tree) would need the same filter to stay consistent with it.
