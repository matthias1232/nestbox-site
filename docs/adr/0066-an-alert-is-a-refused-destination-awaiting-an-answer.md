# ADR 0066 — An alert is a refused destination awaiting an answer

Status: Accepted
Date: 2026-11-02


## Context

Operators wanted red events to pop up and be answerable at once ("allow now", "allow 15
minutes"). The platform already stores every network, DNS and MCP decision as telemetry
and already has the primitives to answer one: a one-step allow rule, an evaluation
window, a rule draft. What was missing was a definition of "needs attention" and a way
to push it.

## Decision

* **An alert is derived, not stored.** It is a refused destination (sandbox, host, port)
  seen in a recent window (default 15 minutes) that nobody answered: no rule and no open
  window has covered it since its last occurrence. `GET /alerts` computes it from the same
  decision data the traffic page reads, grouped by destination, with a count and first/last
  time. The id is a hash of sandbox, host and port, so it is stable between calls.
* **It is published live from the ingest path.** Where `telemetry_service` stores a
  blocked or refused network or DNS event, it also emits an `alert` event on the live
  stream with the same shape (count may be 1). The console's bell, dock tab and popup
  read both.
* **Four answers, all existing primitives.** *Allow now* creates the permanent one-step
  allow rule; *allow for a time* opens a bypass window with a reason; *make a draft*
  creates a rule draft from the event; *ignore* is client-side for the session and is not
  persisted. Answering changes the state the next derivation reads, so the alert vanishes
  without a "resolve" step.
* **Visibility follows the sandbox.** A member sees alerts for their own sandboxes only.

## Why no alert table

A table would be a second source of truth for "is this destination allowed", and it would
drift from the rules and windows the moment someone answered through another page. A
derived alert cannot be stale in that way; its cost is one grouped query, bounded by the
window.

## Alternatives rejected

* **Persisted alerts with an acknowledged flag.** Needs cleanup, races with rule edits and
  doubles the audit story.
* **Alerting on every blocked event.** A noisy sandbox would bury the rest; grouping by
  destination keeps one alert per thing to decide.
* **Server-side ignore.** Ignoring is a view preference; making it shared state would hide
  a destination from colleagues who should still decide.

## Consequences

An alert that was answered by a time-limited window returns when the window ends and the
destination is refused again, which is the intent. `would_deny` (evaluation windows) is
carried so the console can colour it differently from a real refusal.

## Not built

Notification channels outside the console (email, chat), and per-user alert rules.

## Invalidation conditions

If alerts need history beyond the telemetry retention (reporting, SLAs), a summary table
is added as a read model fed from the same query; it never becomes the source of truth.
