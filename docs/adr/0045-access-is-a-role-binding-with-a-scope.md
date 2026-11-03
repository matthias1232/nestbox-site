# ADR 0045 — Access is a role binding with a scope, and a sandbox can be delegated

Status: Accepted
Date: 2026-10-31

## Context

ADR 0004 gave three roles per environment. A provider needs a company level, a customer level and an employee level, and must be able to give one person full control of one sandbox only. ADR 0004 names delegation below the environment as the reason to revisit.

## Decision

1. **`role_bindings(user_id, scope_type, scope_id, role)`** replaces the single role as the source of truth. `scope_type` is `platform | organization | environment | site | sandbox`.
2. Roles: `platform_admin`; `org_owner`, `org_admin`; `env_admin`, `operator`, `viewer`; `site_operator`; `sandbox_owner`.
3. A binding applies to everything below its scope. The existing `users.role` and `user_scopes` are migrated into bindings and keep working until the last reader is moved.
4. **`sandbox_owner` may do everything inside one sandbox**: terminal and exec, packages and recreate, the sandbox's own network and DNS rules, rule drafts, traffic windows and master switch for that sandbox, live traffic, MCP, logs, usage, backup and migration. It may not change global policy, see provider keys or manage nodes.
5. **Policy gets a sandbox scope** so a sandbox owner edits only their own rules (the next step ADR 0038 left open).
6. Checks stay in the service layer; row-level security repeats them through `nb_can(scope, permission)`, so a forgotten check is still refused by the database.
7. A test matrix proves that every foreign resource, asked for by a guessed id, answers 404.

## Alternatives rejected

* **More fixed roles.** Every new case becomes a migration of the enum.
* **Per-resource ACL lists.** Flexible, but no one can answer "what can this person touch?" without scanning everything.
* **Application checks only.** One forgotten check is a cross-tenant leak (ADR 0004).

## Invalidation conditions

Revisited when customers need custom roles, or when binding evaluation shows up in query plans.

## Amendment 2026-11-01 - what was built

The design said `role_bindings(user, scope_type, scope_id, role)`. What shipped is smaller and covers
the cases that were asked for, with the database doing the isolation:

* the **member** role (between viewer and operator), granted per environment as before;
* **`sandbox_grants(sandbox, user)`**, which extend the sandbox visibility policy, so telemetry, rules
  and supervisors follow without further code;
* **`SandboxPolicy`**, the permission a member holds for the rules, drafts, traffic windows and egress
  mode of a sandbox they can see; shared policies and the environment-wide switch stay `PolicyWrite`;
* the organisation tree gives provider-level reach.

Site-level roles (`site_operator`) and organisation-level roles below admin are not built; the
general `role_bindings` table is the next step if they are needed. A member's persistent folders are
those they own; a folder cannot yet be granted separately from its sandbox.
