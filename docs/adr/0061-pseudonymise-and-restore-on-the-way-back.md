# 0061 — Pseudonymise on the way out, restore on the way back

## Context

Blocking protects a value and stops the work. Rewriting protects the value and
leaves the agent with `<PERSON_NAMES_1a2b3c4d>` in a file it writes, a path it
opens, or a host it connects to — which is the same as breaking the task for
every detector except the ones whose values the agent never needs (an IBAN, a
card number, a password).

The transparent hop holds the plaintext of both directions, so it can do better:
replace the value on the way to the model, and put it back in the model's answer
before the agent reads it. The model never sees the value, the agent never sees a
placeholder.

## Decision

`pseudonymize` is an action next to `block`, `redact`, `replace` and `allow`.

* **Outward** it behaves exactly like `replace`: the placeholder is
  `<DETECTOR_xxxxxxxx>`, derived from the HBAC fingerprint, so the same value
  yields the same placeholder in every message and the provider's prompt cache
  keeps working.
* **The answer carries the pairs.** Only the internal inspect endpoint returns
  `restore: [{placeholder, value}]`, and only for `pseudonymize` findings. The
  proxy keeps that list in memory for the lifetime of one request. It is never
  logged, never stored, never written to the database; `Debug` on the type prints
  `[redacted]` for the value.
* **Inward** the broker rewrites the answer before it reaches the sandbox:
  * a whole JSON answer: the placeholder inside a string value is replaced by the
    value, escaped for a JSON string;
  * server-sent events: `delta.text` is plain text and a placeholder can be split
    across deltas, so a tail that could still become one is held and prefixed to
    the next delta of the same block; `delta.partial_json` carries a JSON fragment
    and is restored with one more level of escaping. A fragment never holds a
    tail — handing a client half a document is worse than a placeholder that stays.
  * an answer larger than 8 MiB is streamed through unchanged.
* The map needs no storage because an agent resends its whole history with the
  original values on every turn: the current request's pairs are the only ones
  the answer can name.

## What this does not do

* A model that invents a placeholder gets it back unchanged; the row records an
  `outbound` finding with `allowed`.
* The outbound direction is not scanned for new values; only the pairs of the
  current request are restored.
* `forward_proxy` and `direct` cannot be read at all, so nothing is restored
  there — the tab says so.

## Consequences

An agent keeps working with names, hosts and paths it can actually use, while the
provider never receives them. The cost is one JSON walk of the answer and, for
streaming, a small buffer per block. The risk is asymmetric and stated: a
restoring bug shows a placeholder to a user; a leaking bug shows a value to a
provider.
