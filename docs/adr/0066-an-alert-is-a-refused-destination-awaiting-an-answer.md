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

## Addendum — recording off keeps the refusals (2026-11-03)

`recording=off` on the quick controls sets both recording mutes for up to 24 hours
(`quick_control_service::set_recording`). Until now that muted `all`: no network, DNS or
MCP row was stored, so no alert could be derived from history, the tree badge went
green, and the audit trail showed a sandbox that was quiet rather than a sandbox that
was refusing traffic.

The mute is a request to keep no *content*, not a request to hide a decision. So
`telemetry_service::ingest` and `ingest_batch` now keep an event while `all` is muted
when both hold:

* `kind` is `network` or `dns`, and
* the stored `detail.decision` is `deny`.

Such an event is stored as metadata only: the `traffic_bodies` row, the MCP tool-call
row and the chat turn are dropped, and `detail.has_body` is stored as `false`. `allow`
and `audit` flows, inference rows and everything else are still not stored while the
mute runs. The live stream is unchanged: it already published every event, muted or
not, which is why the console's bell could fire while history could not explain it.

The end-user permission is deliberately unchanged: a member may still switch recording
off, because the user asked for that switch (F6 of the 2026-11-03 review). What changes
is that the governance hole closes — an operator keeps the refusals, the alerts, the
tree badge and the audit trail for the duration of the mute. The cost is bounded: at
most one row per refused destination per attempt, and no bytes.

One consequence to keep in mind: an event that arrives without a decision is read as a
deny (`parse_decision`), so a proxy version that stops sending the field keeps storing
its flows during a mute. That is the same conservative default the traffic log already
uses.

## Not built

Notification channels outside the console (email, chat), and per-user alert rules.

## Invalidation conditions

If alerts need history beyond the telemetry retention (reporting, SLAs), a summary table
is added as a read model fed from the same query; it never becomes the source of truth.
