# privacy — the content-privacy (DLP) layer

## Purpose

One screen for the question "what is inspected on its way into and out of a
sandbox, and what happens to a finding?". The top half is the policy for the
environment default or one sandbox; the bottom half tests that policy once and
shows the incidents it recorded.

## Files

| File | Responsibility |
|---|---|
| `PrivacyPage.jsx` | Scope picker, mode, detectors with their actions, egress and retention, the inspection test, incidents, the admin purge |
| `DlpPanels.jsx` | The DLP dashboard (`DlpDashboard`: findings grouped by detector, action, severity, sandbox, day or destination), the log (`DlpLog`: direction, detector, what happened, masked value → substitute, the request, the hop that saw it; filterable) and the policy history (`DlpChanges`: who changed what, with the old and the new values). All take an optional `sandboxId`; without one an admin sees the environment, anyone else only the sandboxes they own |
| `usePrivacy.js` | Loading, the draft, saving, inspecting, purging, error text; no JSX |

## Rules

1. A blank sandbox means "environment default", never "every sandbox". The picker
   has an explicit option for it, and inspection refuses to run without a
   sandbox, because the sandbox's policy is what answers.
2. The mode is a decision, not a label: each mode is offered with one sentence
   that says what it does, and no sentence claims more than the server does.
3. `allow` is not hidden. The page says plainly that the server refuses it for a
   secret detector, so a refused save is expected rather than surprising.
4. The inspection box tests the filter; it is not a conversation. The page says
   so, and the result shows the transformed text and the masked findings instead
   of implying that something was sent.
5. Incidents carry masked values only. The page states that the original value
   is never stored, and never renders a field that would suggest otherwise.
6. Saving is explicit and admin-only. A changed select is not applied until
   "Save", because the environment default changes what every sandbox inherits.
7. The purge asks once and says what it deletes: incident rows older than the
   day count, for the selected sandbox.

## Wiring

`App.jsx` is not part of this package: the integrator adds the route and the
`features/AGENTS.md` row. The page takes a `role` prop and hides the save and
purge controls for anyone who is not an admin.

## The three facts a row must keep apart

`action` is what happened, `requested_action` is what the policy asked for, and
`replacement` is what the filter wrote instead of the value. In `observe` the
first two differ — the row says "recorded only · would have refused" — and that
difference is the only evidence that content left after a match. Never collapse
them into one word: "allowed" alone reads as "nothing was found".
| `PrivacyDlpSection.jsx` | The DLP section of the Content privacy page |
| `PrivacyExtraSettings.jsx` | The two environment switches for exceptions and chat reading |
| `DlpFindingDrawer.jsx` | One finding in full, with the way to allow it on purpose |
| `DlpExceptionsTable.jsx` | The findings allowed on purpose, and when they stop |

Restyle (F25): `PrivacyPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
