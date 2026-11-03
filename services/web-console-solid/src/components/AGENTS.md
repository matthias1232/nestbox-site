# components/ — reusable UI

## Purpose

Building blocks that know nothing about sandboxes, policies, or models. A
component here takes data through props and emits events through callbacks.

That rule is what makes them reusable: `TopologyGraph` renders nodes and edges,
not gateways and sandboxes.

## Rules

1. No fetching. A component that fetches cannot be used in two places with
   different data.
2. Props in, callbacks out. No global state reads.
3. One job per file, named for what it looks like or does.
4. No business rule. "A sandbox may only be stopped while running" belongs in
   the domain, and the component receives `disabled`.
5. No arithmetic that decides something. `ResourceBars` computes pixel widths
   from server numbers; it must not decide whether a model fits. That decision
   arrives as `status` and `fit`.

## Files

| File | Responsibility |
|---|---|
| `StatusBadge.jsx` | A coloured label for a status string |
| `Drawer.jsx` | A side panel with a title and a close action |
| `TopologyGraph.jsx` | Layered node/edge diagram (gateway → sandbox → destination), with blocked traffic drawn solid and clickable |
| `TrafficTable.jsx` | A dense event table over normalised rows: sandbox, decision, channel, destination, request, reason |
| `FitBadge.jsx` | Model fit verdict with its reason |
| `ResourceBars.jsx` | "Have versus need" bars for VRAM, RAM, cores; draws server numbers |
| `CapacityOverview.jsx` | The machine's own CPU, RAM, and GPU capacity, scaled against the catalog |
| `ModelFitCell.jsx` | Badge, compact bars, and the shortfall in words |
| `Tabs.jsx` | Accessible tab strip |
| `Empty.jsx` | A consistent empty state |
| `LoginBadge.jsx` | One sandbox's sign-in state, one mode per agent when the entry carries `agents`, else the single summary badge |
| `LogViewer.jsx` | One log panel for a sandbox, a supervisor (filtered) and an appliance component: follow, filter, copy; stops following after a failed read |
| `PendingPanel.jsx` | A tab body that a later package fills: what belongs there, which endpoint, who owns it |
| `Switch.jsx` | A two-state switch, a hidden checkbox under a styled track |
| `Segmented.jsx` | A radio group of styled buttons, one active |
| `Countdown.jsx` | A live "time left" label; no end time reads as "unlimited" |
| `countdown.js` | Time formatting behind `Countdown`, pure and node-tested |
| `ConfirmDialog.jsx` | A centred modal whose buttons the caller names |
| `Banner.jsx` | A one-line notice coloured by tone |
| `PresenceDots.jsx` | Who is in a sandbox, as up to five initials plus "+N" |

Workbox primitives: `Switch`, `Segmented`, `Countdown`, `ConfirmDialog`, `Banner`, `PresenceDots`.
They hold no rules: the caller decides, the component draws. Their shared styles live in
`styles/controls.css`, which each of them imports.
| `dataTable.js` | Sort, filter, search, group and CSV as pure functions |
| `DataTable.jsx` | The shared table: toolbar, filters, groups, export, live rows |
| `DlpBadge.jsx` | One badge per content-check state, in the shared colours |
| `dlpText.js` | Split a masked text into replaced, pseudonymised and unknown parts |
| `DlpText.jsx` | A masked text with its findings marked |
| `RouteChain.jsx` | The way a request took, or would have taken |
| `liveTicker.js` | A bounded, keyed list of live events |
| `dataTable.test.js` | Sort, filter, search, group and CSV behaviour, including nulls, dates and CSV quoting |
| `dlpText.test.js` | Splitting a masked text into marked parts and recorded-only findings |
| `liveTicker.test.js` | Bounded size, merging by key, time order and removal |
| `QuickAllow.jsx` | Four buttons that allow one denied destination for 15 min, 1 h, 24 h or permanently |

Control Center placeholders (Wave 0; each renders a "coming in package" note until its package builds it, props in
contract C15): `icons.jsx` (F02; only `IconHome` yet), `IconButton`, `StatTile`, `Gauge`, `Sparkline`, `KeyHint`,
`Toast`, `Tree`, `Wizard`, `HelpTip` (F02), `SelectTable` (F02: the selectable, sortable table of contract C15;
named `SelectTable` because the existing `DataTable.jsx` is used by many pages and keeps its own props),
`CommandPalette` (F04), `AlertPopup` (F13), `TopologyCanvas` (F16), `ConsoleFrame` (F20), `BulkBar` (F21).
`markdown.js` is the dependency-free renderer (F18).

Control Center primitives (package F02; styles in `styles/primitives.css`):

| File | Responsibility |
|---|---|
| `icons.jsx` | Every inline SVG icon (`IconNode`, ...) plus `Icon({ name })` |
| `IconButton.jsx` | A 28x28 icon-only button with tooltip and accessible name |
| `StatTile.jsx` | Eyebrow label, big mono figure, unit and trend; a button when clickable |
| `Gauge.jsx` / `gauge.js` | 270 degree SVG arc and its arc maths |
| `Sparkline.jsx` / `sparkline.js` | SVG polyline trend and the points calculation |
| `KeyHint.jsx` | `<kbd>` rendering of a shortcut such as `mod+k` |
| `Toast.jsx` | The toast stack, reading `ui.toast.queue()` |
| `Tree.jsx` / `tree.js` | Keyboard-driven inventory tree with windowing over 400 rows; flatten, filter, cursor helpers |
| `SelectTable.jsx` / `table.js` | Selectable, sortable table (the older `DataTable.jsx` stays); sort and selection helpers |
| `Wizard.jsx` | Step list, body and Back / Next / Finish footer |
| `HelpTip.jsx` | A "?" button that opens the help drawer at an anchor |
| `gauge.test.js`, `sparkline.test.js`, `tree.test.js`, `table.test.js` | Node tests for the helpers |
F04: `CommandPalette.jsx` is the search dialog (props in, `onOpen(path)` out). `palette.js` holds its pure logic (matching, grouping, filters, arrow movement, stale-answer counter); `palette.test.js` tests it.
