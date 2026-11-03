# policies/ — policy documents and rule approval

## Purpose

Edit the four-section policy, see what a change will actually do, and approve
rule drafts.

## The distinction this screen must never hide

Only `network_policies` applies to a running sandbox. The other three sections
need a recreate. The save response says which happened (`applies_now`,
`requires_recreate`, `rejected`), and this screen shows that answer rather than
a generic "saved".

## The inventory

The list is the inventory of every policy that exists, with two derived facts
that decide where an operator has to look:

- **kind** — `openshell` when the document carries a startup section
  (filesystem, Landlock, process), `network` when it carries only
  `network_policies`. Derived from the document, never stored, so an edit cannot
  leave a wrong badge behind. The server computes it; the console filters and
  searches on it.
- **history** — every save writes a revision before it applies, so "what was this
  policy" is a read, not a reconstruction. It is listed per policy, newest first,
  with who saved it and when.

## Rule drafts

A draft is not a rule. Approval is the only path that writes a rule, which is
what makes the workflow meaningful (ADR 0011).

`PolicyWizard.jsx` is the placeholder of the policy wizard (F14).
