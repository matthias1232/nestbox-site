# master — the organisation view

## Purpose

One screen for "what did this organisation spend, on which model, in which
sandbox, and what did the content filter do on the way?". The top half is the
environment's window; a sandbox in the breakdown narrows the whole screen to
that sandbox, and one control goes back to the organisation.

## Files

| File | Responsibility |
|---|---|
| `MasterControlPage.jsx` | Timeframe control, KPI row, by-model and by-sandbox bars, privacy panel, request table, admin retention and purge |
| `useMasterControl.js` | Timeframe, drill-down, the two reads, the retention draft, the purge; no JSX |
| `BarList.jsx` | One horizontal bar per group; draws the server's numbers, no arithmetic that decides anything |
| `accountingFormat.js` | Display rules: counts, cost, status tone, privacy actions; pure, no signals |

## Rules

1. **A missing price is `n/a`, never `0`.** `cost_micros = null` renders
   "n/a (no price)", and a window with unpriced requests says how many, because
   a partial sum presented as a total is the one number that must not be shown.
   The same rule applies to a single row's cost.
2. **A drill-down is a filter, not a route.** It sets the same sandbox on both
   reads and is cleared by one control; a reload returns to the organisation
   view, like every screen except the Admin Center.
3. **The table is a page, not the window.** Newest first, bounded by
   `REQUEST_LIMIT`, and the page says so. Sorting is repeated here so the
   "newest first" promise survives an implementation that does not sort.
4. **Privacy actions are explained, one sentence each.** An action the console
   does not know is shown with that admission instead of a guessed meaning, and
   its count is never dropped. `not_inspectable` is kept apart from `observed`:
   unchecked content is not clean content.
5. **Loading, empty and error are separate states.** A failed request list
   never blanks the summary, and vice versa: they are two reads.
6. **Writes are admin-only.** Retention save and purge; everyone else reads the
   saved values as text. The page takes a `role` prop.
7. **The purge asks once and says what it deletes.** Raw request rows older
   than the day count; the daily rollup survives when the setting says so.
8. **No URL in this folder.** Every call goes through `api/accounting.js`.

## Wiring

`App.jsx` and `features/AGENTS.md` are not part of this package: the integrator
adds the route and the row. The page is mounted with the session role:

```jsx
import MasterControlPage from './features/master/MasterControlPage.jsx';
// ROUTES: { id: 'master', label: 'Master control' }
<Show when={route() === 'master'}>
  <MasterControlPage role={session.identity()?.role} />
</Show>
```

Restyle (F25): `MasterControlPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
