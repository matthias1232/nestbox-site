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

## Policy wizard (F14)

| File | Role |
|---|---|
| `PolicyWizard.jsx` | The `Wizard` page: loads the policy, walks the steps, runs `applyPlan`. Also holds the small Filesystem and Tools steps. |
| `policyWizard.js` | Pure logic: `emptySpec`, `stepsFor`, `addAllow`, `removeEntry`, `setDns`, `setProcess`, `validateStep`, `applyPlan`, `runPlan`, suggestions. Tested in `policyWizard.test.js`. |
| `NetworkStep.jsx` | Allow list table, quick add, suggestions from the last 24 h of decisions. |
| `DnsStep.jsx` | Allow and block patterns; saved as DNS rules of the policy, not in the document. |
| `ProcessStep.jsx` | Process fields and Landlock paths (apply after a recreate, ADR 0008). |
| `ObservationStep.jsx` | Proposes an egress mode and an evaluation window; Review lists the calls. |
| `ReviewStep.jsx`, `CompiledView.jsx` | Name, scope, compiled YAML, skipped rules, hot/cold badges, diff, and the calls Apply makes. |

`PoliciesPage` opens the wizard from "New policy (wizard)", "Edit in wizard" and `?wizard=1&sandbox=<id>`.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `PoliciesPage.jsx` | Policies, network rules, and the draft queue. Every tab keeps its table header and puts the empty state under it (`styles/policies.css`): an empty state may not sit in the grid column beside the header, where it reads as a column of the table (V27). |
| `PolicyForm.jsx` | A policy document as a form, rendered from `/policy-schema`. |
| `RuleForms.jsx` | Forms for a network rule and a DNS rule. |
| `SandboxPicker.jsx` | Choose which sandboxes something is applied to: some of them, or all. |
| `policyForms.js` | Pure helpers for the policy forms: no JSX, no network. The server owns what is |
| `policyForms.test.js` | Node tests for `policyForms`. |
| `ruleHelp.js` | What each field of a rule means, what it accepts and an example, in one place. |
| `ruleHelp.test.js` | Node tests for `ruleHelp`. |
