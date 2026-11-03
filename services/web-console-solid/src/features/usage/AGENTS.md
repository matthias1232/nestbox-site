# features/usage — what the models used, and what it cost

| File | What it does |
|---|---|
| `UsagePage.jsx` | The environment-wide "Usage & cost" page. |
| `UsageView.jsx` | The table and totals, grouped by model, provider, sandbox, key/account, owner, data center, customer, agent, conversation or day. Shared with the sandbox deck's Usage tab. |
| `PriceList.jsx` | The price list, the models used without a price, and a form to set one. Saving prices the calls already recorded. |
| `format.js` | Pure display rules: compact tokens, `n/a` for a missing price (never 0), percentages. |

Two rules the screen keeps: a missing price is `n/a`, and what an API key cost is
never added to what a signed-in account (Claude, Codex) would cost at list price.
The numbers come from `GET /accounting/usage`, filled by the egress proxy reading
model answers (inspecting modes) and by the accounting proxy.

Restyle (F25): `UsagePage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `format.test.js` | Node tests for `format`. |
