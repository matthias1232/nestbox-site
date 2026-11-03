# 0062 — A finding can be allowed on purpose, and an exception is a row, not a mode change

## Context

The privacy layer's defaults are deliberately loud: `strict` blocks an IBAN, a
card number, an API key. In real work the same value then comes back — a test
IBAN in a fixture, a customer's address in a support conversation, a hostname
that is also a person's name by accident. The operator's options were bad:
switch the whole environment to `review`, which stops blocking everything, or
turn the detector off, which stops recording everything. Both are permanent
policy changes made to get one request through.

What is needed is a decision about *one finding*, recorded, with an end, that
leaves the policy alone.

## Decision

A new table, `privacy_exceptions` (`0083_privacy_exceptions.sql`), and a
matching step in the evaluation of every finding.

**The row.** `environment_id`, `sandbox_id NULL` (NULL = the environment),
`detector`, `value_fingerprint NULL` (NULL = any value of that detector),
`destination NULL` (host), `action` (`allow`, `replace`, `pseudonymize`),
`expires_at NULL`, `reason`, `created_by`, `created_at`. Row level security and
the tenant policy are the same as the other privacy tables: an exception is
tenant data, and every query filters by `environment_id`.

**The match.** For each finding, the exceptions of the environment are loaded
once per request (not per finding) and the most specific one wins:

1. **Sandbox before environment** — a rule for the sandbox the request came from
   beats the environment's default. The narrower statement is the one somebody
   made about this case.
2. **Value before detector** — an exception naming a fingerprint beats one that
   covers the whole detector. Naming a value is a stronger statement of intent
   than naming a class.
3. **Destination before no destination** — an exception for one host beats one
   that applies everywhere.
4. **Not expired**, the same `expires_at IS NULL OR expires_at > now()`
   predicate the rest of the appliance uses (ADR 0060). Tie-break on the most
   recently created row, so the later decision wins and the outcome is
   deterministic.

The winning exception's `action` replaces the configured action for that
finding. The incident records `exception_id`, so the console can say "allowed by
an exception" instead of "clean" — an exception must never look like a detector
that found nothing.

**It only works through the fingerprint.** An exception that named a value in
plaintext would be the one table in the appliance holding content, and the whole
"no column for the value" argument would end there. Matching goes through the
salted fingerprint (ADR 0061's HMAC, the same one the placeholder is built
from). That is also why exceptions depend on the salt: with the unsalted SHA-256
of a name or a phone number, a dictionary recovers both the fingerprint and the
value, and the table becomes a guessable list of secrets.

**Who may grant one.** An administrator always. The sandbox's owner only while
`owners_may_grant_exceptions` is on in the environment's privacy settings
(default off) — an owner can already send whatever they like, so the permission
is about granting *others'* traffic an exception, not their own. A
non-privileged caller, and a caller outside the tenant, gets `404`: the
existence of a sandbox is not information they are owed. Every grant and every
removal writes `privacy.exception` to the audit log.

**How it is reached.** From a blocked row: `POST /api/v1/privacy/exceptions`
with `{incident_id, scope: "value" | "detector", sandbox_only, action, minutes,
reason}` — the incident supplies detector, fingerprint, sandbox and destination,
so the caller never types a value and the console never has to show one. `GET`
lists them, `DELETE /{id}` removes one. `minutes` is `60`, `1440` or `null` for
no end.

## Consequences

* A block can be excused without weakening the policy: the environment stays
  `strict`, the next different IBAN is still refused.
* An exception is evidence. It says who allowed what, why, since when and until
  when — which a mode change never did.
* The evaluation grows one lookup per request and a small "most specific wins"
  decision per finding. The list is per environment and short; if it ever is
  not, the index on `(environment_id, sandbox_id)` is the first thing to look at.
* Deleting an exception does not rewrite history: incidents that were allowed
  keep their `exception_id` and their reason.
* An expired exception needs no cleanup to be harmless, but the reconcile pass
  deletes it for the same reason it deletes expired rules.

## Rejected alternatives

* **A mode switch per sandbox.** Too blunt: it unblocks every detector and
  leaves no trace of why.
* **A per-detector "ignore" list.** The same value returns tomorrow, and the
  list is permanent.
* **Exceptions keyed by incident.** An incident is one occurrence; the next
  identical request would be blocked again, which is exactly the loop the user
  is trying to leave.
* **Exceptions holding the value.** It inverts the module's central promise and
  turns the exception table into the most sensitive table in the appliance.
* **Silently allowing a repeat of an already-allowed fingerprint.** Without a
  row there is no owner, no end, and no one to ask. The row is the point.

## What would invalidate this

A need for exceptions that differ per user rather than per sandbox would add a
`user_id` dimension and change the specificity order. An appliance whose
fingerprint key rotates would need the fingerprint to carry its key generation,
or exceptions (and findings) would stop matching after a rotation — that is a
deliberate follow-up decision, not a bug to patch in place.
