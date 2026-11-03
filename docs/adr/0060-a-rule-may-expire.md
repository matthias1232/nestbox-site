# 0060 — A rule may expire, and the expiry lives in the database, not in the struct

## Context

The most common answer to a blocked request is "let it through this once, now".
Doing that with a permanent rule leaves an allow nobody remembers to remove, and
on an appliance whose default is deny, every forgotten allow is a hole with a
comment. The live view therefore offers an allow that ends by itself, and
`network_rules` grew a column for it (`0085_rule_expiry.sql`).

An expiry has to be honoured in four places that read rules for different
purposes: the decision path (does this request pass), the compilation of the
OpenShell policy (what the gateway is told), the list and detail screens (what
an operator sees), and the quick-allow list (what this sandbox has open right
now). One missed place is enough to keep an expired allow alive.

## Decision

`expires_at timestamptz` on `network_rules`; `NULL` means the rule does not
expire.

* **Every read that can decide, compile or display filters it out**:
  `expires_at IS NULL OR expires_at > now()`. The predicate sits in the query,
  not in a service, so a new caller cannot forget it without writing its own
  SQL.
* **Deletion is the reconcile pass's job.** `quick_allow::expire_due` deletes
  expired rows once per pass (30 s), through the same service that a manual
  delete goes through — so the cache is invalidated and the gateway policy is
  pushed exactly as it is for a delete the operator asked for.
* **Deletion is garbage collection, not the mechanism.** A rule whose moment
  passed but which has not been deleted yet is already invisible to every
  filter. The reconcile pass only keeps the table from filling with dead rows
  and stops `expires_at`-based lists from growing without bound.
* **The field is not on `NetworkRule`.** The struct is the compiled, in-memory
  form a decision uses. A rule that reached that struct has already been decided
  to be current; carrying an `Option<DateTime>` there would invite `if
  rule.expired()` checks in some paths and nothing in others, which is precisely
  the inconsistency this decision avoids. The expiry is a property of the row,
  read by the query and by the row-shaped API types (`QuickAllowRule`), and the
  console shows it as "until 14:32".
* **An index keeps the pass cheap**: `network_rules_expiry_idx` on
  `expires_at` where it is not null.

## Consequences

* A quick allow cannot outlive its moment even if the control API restarts or
  the reconcile pass is stuck: filtering happens per read.
* A gateway restart recompiles without the expired rule, because compilation
  reads through the same filter. No separate step is needed.
* The delete is idempotent and failure is logged, not fatal: a skipped pass
  leaves rows that are already ignored.
* The UI must not treat "exists" as "applies". The quick-allow list selects on
  `expires_at > now()` so a row cannot be shown as open after its end.
* Two clocks now matter: the database's `now()` for filtering and deletion, and
  the browser's for the countdown. The database wins; the countdown is a hint.

## Rejected alternatives

* **An `active` boolean, flipped by a timer.** It needs the same filters
  everywhere plus a writer, and it loses *when* the rule ended — which is what
  an audit asks.
* **Expiry only in a side table.** It splits the rule's lifetime from the rule,
  and every read would need a join that some read forgets.
* **Refusing every rule without an expiry.** An operator's deliberate permanent
  allow is legitimate; the point is that a *temporary* allow must be able to say
  so.
* **Deleting on read.** Turns a list call into a write, which breaks read
  replicas, and a request that is never made never cleans up.

## What would invalidate this

A rule that must outlive its own moment — a staged policy that starts later —
would need a `starts_at` beside `expires_at`, and the predicate becomes a range
check. A decision path that cannot afford the filter (a hot loop over a cached
list) would need an in-memory compile that carries the expiry; that is the case
in which the struct grows the field, together with a test that the compiled
form is rebuilt when the pass deletes rules.
