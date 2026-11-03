# ADR 0054 — "Allow everything" states are loud, bounded by choice

Status: Accepted
Date: 2026-11-01

## Context

ADR 0041 bounded the master switches at 24 hours and made the OpenShell channel
unswitchable, because switching the supervisor off cuts a sandbox off from its tools
rather than opening traffic. Both limits were right for a switch, and both turned out
to be wrong for the job people actually do with a workbox: install something, let an
agent work, watch what happens — sometimes for an afternoon, sometimes until further
notice, and sometimes including the tooling itself.

The product answer is that the user must be able to allow everything and to say for
how long. The risk answer is that an unbounded allow must never look like a normal
state.

## Decision

**Two ways to allow everything, and both are visible.**

* **Egress mode `off_allow_all`** — persistent until it is switched off. The proxy
  tunnels all traffic and records host, port and bytes; rules are not enforced, and
  OpenShell receives its allow-all document. It is a normal user-selectable mode, not
  an admin switch.
* **Traffic windows** (ADR 0041) — `bypass` (allow quietly) or `audit` (allow and
  record the decision as a block), per channel (`proxy`, `dns`, `mcp`, `gateway`,
  `openshell`), for a number of minutes or **unlimited** (`expires_at = NULL`, until
  it is closed).

**Both record `would_deny` next to every allow.** Rules are evaluated first; a
covering window then answers allow or audit. So `would_deny` is the rule's verdict,
not a second opinion, and an allow that a rule would have denied stays available for
a draft rule later (ADR 0011).

**The loudness is part of the decision.** Workbox users see a permanent warning
banner while a sandbox is in one of these states; the admin list badges it; the
control summary reports `allow_all` and `allow_all_until`; the proxy access log
carries `would_deny`; the traffic rows mark an allow as observed only.

**An environment may bound it.** `environments.max_window_minutes` (default: none)
rejects an unlimited window or a longer one with 422. The sandbox's own mode
`off_allow_all` is not bounded by it, because it is a persistent setting with a
visible switch, not a window that is easy to forget.

## Alternatives rejected

* **A hard 24-hour cap.** It is the user's sandbox and the user's decision. The
  bounded window exists for the caution case, not for every case.
* **Silently allowing.** An allow that hides that a rule disagreed is the failure
  mode this ADR exists to prevent: `would_deny` is the difference between "this was
  fine" and "this would have been blocked".
* **Auditing only the proxy channel.** The OpenShell supervisor holds the tooling; a
  mode that opens the network but not the tools does not remove the sandbox's limit,
  and one that opens both must say so.
* **Making `off_allow_all` an admin-only setting.** It would push every workbox user
  into asking an admin for the state they can already describe.

## Consequences

A user can put a sandbox in a state where nothing is enforced, and the console says so
on every page that shows the sandbox; see ADR 0053 for why switching the mode to
`off_allow_all` needs no recreate. The traffic log keeps working in that state, so
the evidence is not lost while the rule is absent. An administrator can cap windows per
environment when a tenant asks for it, and the cap is reported as an error message,
not as a silent shortening.

## Invalidation conditions

When the OpenShell supervisor cannot actually allow traffic that its audit path was
told to allow, the `openshell` channel would be observation only rather than a bypass;
that has to be stated in this ADR instead of being inferred from the code.
