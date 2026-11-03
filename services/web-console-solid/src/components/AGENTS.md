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
6. User-visible text is English (P08): labels, tooltips, empty states and the
   CSV file name. `DataTable`, `DlpBadge`, `DlpText` and `RouteChain` were the
   last ones translated; `src/noGerman.test.js` fails if German characters or a
   short word list return.

## Files

| File | Responsibility |
|---|---|
| `StatusBadge.jsx` | A coloured label for a status string |
| `Drawer.jsx` | A side panel with a title and a close action |
| `TopologyGraph.jsx` | Layered node/edge diagram (gateway → sandbox → destination), with blocked traffic drawn solid and clickable |
| `TrafficTable.jsx` | A dense event table over normalised rows: sandbox, decision, channel, destination, request, reason; sticky header, decision tone word, `onAnswer(row, answer)` for refused rows |
| `FitBadge.jsx` | Model fit verdict with its reason |
| `ResourceBars.jsx` | "Have versus need" bars for VRAM, RAM, cores; draws server numbers |
| `CapacityOverview.jsx` | The machine's own CPU, RAM, and GPU capacity, scaled against the catalog. Its card keeps its own content: long machine values (the CPU model, a host name, a GPU name) wrap inside the card instead of running over the next one (V19; the rules live in `styles/models.css`) |
| `ModelFitCell.jsx` | Badge, compact bars, and the shortfall in words |
| `Tabs.jsx` | Accessible tab strip; the active tab carries the 2px lime underline, a tab may carry a numeric `count` drawn as muted mono after the label (V08) |
| `Empty.jsx` | A consistent empty state |
| `LoginBadge.jsx` | One sandbox's sign-in state, one mode per agent when the entry carries `agents`, else the single summary badge |
| `LogViewer.jsx` | One log panel for a sandbox, a supervisor (filtered) and an appliance component: follow, filter, copy; stops following after a failed read |
| `PendingPanel.jsx` | A tab body that a later package fills: what belongs there, which endpoint, who owns it |
| `Switch.jsx` | A two-state switch, a hidden checkbox under a 34 × 18 px track with a 14 px knob: lime when on, muted when off, a visible focus ring, and the state stays readable when the control is disabled (V26; the rules live in `styles/controls.css`) |
| `Segmented.jsx` | A radio group of styled buttons, one active |
| `Countdown.jsx` | A live "time left" label; no end time reads as "unlimited" |
| `countdown.js` | Time formatting behind `Countdown`, pure and node-tested |
| `bulkActions.js` | Pure bulk-bar rules: `actionsFor(kind, role)`, `paramsValid`, `paramsFor`, `confirmText`, `needsDialog` (F21) |
| `bulkActions.test.js` | Tests for `bulkActions.js` |
| `ConfirmDialog.jsx` | A centred modal whose buttons the caller names |
| `Banner.jsx` | A one-line notice coloured by tone |
| `PresenceDots.jsx` | Who is in a sandbox, as up to five initials plus "+N" |

Workbox primitives: `Switch`, `Segmented`, `Countdown`, `ConfirmDialog`, `Banner`, `PresenceDots`.
They hold no rules: the caller decides, the component draws. Their shared styles live in
`styles/controls.css`, which each of them imports. `Switch` is 34 × 18 px with a 14 px knob
(V26): the track keeps its colour (lime on / inset off) when the control is disabled, because a
half-faded lime bar with a floating square was unreadable on the Users page.
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
contract C15): `icons.jsx` (F02, the complete icon set), `IconButton`, `StatTile`, `Gauge`, `Sparkline`, `KeyHint`,
`Toast`, `Tree`, `Wizard`, `HelpTip` (F02), `SelectTable` (F02: the selectable, sortable table of contract C15;
named `SelectTable` because the existing `DataTable.jsx` is used by many pages and keeps its own props),
`CommandPalette` (F04), `AlertPopup` (F13), `ConsoleFrame` (F20), `BulkBar` (F21).
`markdown.js` is the dependency-free renderer (F18).

U10b/U10c note: `MonitorTabs.jsx` and `DisplayArrange.jsx` import `features/sandboxes/displayLayout.js`,
which is pure (no state, no fetching, no DOM). That is the same exception `TerminalTab` makes for
`features/workbox/sessions.js`: a second copy of the layout rules would be a second truth, and the server
refuses with exactly these sentences.

Control Center primitives (package F02; styles in `styles/primitives.css`):

| File | Responsibility |
|---|---|
| `icons.jsx` | Every inline SVG icon (`IconNode`, ..., `IconBellOff` for muted alert pop-ups) plus `Icon({ name })`; the one source for the sidebar menu, the palette kind glyph, the topology card glyph and the login eye (X10 — no private stand-ins in a feature) |
| `IconButton.jsx` | A 28x28 icon-only button with tooltip and accessible name |
| `StatTile.jsx` | Eyebrow label, big mono figure, unit and trend; a button when clickable |
| `Gauge.jsx` / `gauge.js` | 270 degree SVG arc and its arc maths; optional `text` is the reading drawn under the arc in mono (V09) |
| `Sparkline.jsx` / `sparkline.js` | SVG polyline trend and the points calculation |
| `KeyHint.jsx` | `<kbd>` rendering of a shortcut such as `mod+k` |
| `Toast.jsx` | The toast stack, reading `ui.toast.queue()` |
| `Tree.jsx` / `tree.js` | Keyboard-driven inventory tree with windowing over 400 rows; flatten, filter, cursor helpers. A row of kind `organization` is a section label: its name carries the `eyebrow` face and no status dot (V04) |
| `SelectTable.jsx` / `table.js` | Selectable, sortable table (the older `DataTable.jsx` stays); sort and selection helpers. Style (V10): muted normal-weight header over a 1 px line, 32 px rows, no zebra; ids, addresses and modes carry `.mono` in the column render |
| `Wizard.jsx` | Step list, body and Back / Next / Finish footer |
| `HelpTip.jsx` | A "?" button that opens the help drawer at an anchor |
| `gauge.test.js`, `sparkline.test.js`, `tree.test.js`, `table.test.js` | Node tests for the helpers |
F04: `CommandPalette.jsx` is the search dialog (props in, `onOpen(path)` out). Each row draws an `Icon` for its kind (X10). `palette.js` holds its pure logic (matching, grouping, filters, arrow movement, stale-answer counter); `palette.test.js` tests it.
- `AlertPopup.jsx` — one refused destination with its answers, card or table row (F13); the card is V11: eyebrow `REFUSED · <age>` in `--bad`, one line `<sandbox> → <host>:<port> (<channel>, N×)`, frame `1px solid var(--bad)`, and the four answers `Allow 15 min` (primary) · `Allow now` · `Draft rule` · `Ignore`. With `onHide` the card also draws a `×` (`aria-label="Hide this pop-up"`) that hides this pop-up only, leaving the alert open and the bell counting (U05). `alertAnswers.js` holds the pure answer list (used by the table row) and request bodies (tested).
Topology (F16): `TopologyCanvas.jsx` is the layered SVG drawing with zoom, pan, traffic-window switch and legend;
each card draws an `Icon` per kind from `icons.jsx` inside a `translate(10, 10)` group (X10);
`TopologyPanel.jsx` is its side panel (facts, receives/sends lists, Open/Policies/Logs, Allow 15/30/60 on a refused
destination); `Minimap.jsx` is the 160 px overview; `topologyLayout.js` is the pure layout, edge path, fit, zoom and the
`fromCluster` / `fromStack` mappers (`node --test topologyLayout.test.js`). Styles: `styles/topology.css`.
Layout contract (V22): the stage is one flex row of the drawing column (`.topo-view`: SVG, toolbar, legend), the minimap
and the side panel, so the minimap has a reserved corner and can never cover a card — it is outside the SVG on purpose,
because a reserve inside the scaled viewBox would shrink below the minimap's fixed 160 px. `layout` reports `width` that
covers every card's `x + card.w` (tested), and the column titles own a `band` above the first card (`top`, `titleY`).
| `ConsoleFrame.jsx`, `consoleFrame.js`, `consoleFrame.test.js` | one console per sandbox: Shell/Desktop/SSH split button, toolbar (font, copy, paste, fullscreen, Ctrl+Alt+Del, view-only, pop-out); pure `modes`, `pickMode`, `nextFont`, `sshLines` |
| `VncSidebar.jsx` | the desktop's Proxmox-style sidebar: **Display** (the arrangement, then the live status line), Clipboard (send/clear), Full screen, Settings (scaling local/remote/none, view only, dot cursor, shared mode, quality and compression 0–9, automatic reconnect and a 1–60 s delay), Send key (Ctrl+Alt+Del, Ctrl+Alt+F1…F12, Tab, Esc, sticky Ctrl/Alt/Win/Shift), Power (stop, restart, reset = recreate, each behind a `ConfirmDialog` that names the sandbox). Collapsible to a 32 px icon strip; props in, callbacks out. The same file exports `VncStatus`, the line under the picture (`Connected (1440×900, quality 6)` / `Disconnected — <reason>` with Reconnect and Disconnect). Styles: `styles/vnc.css` (U04, Display section U10c) |
| `MonitorTabs.jsx` | the desktop's monitor tabs: `All`, one tab per monitor with its size in mono and a `●` for the primary one, and `+`. `⋯` on a tab opens Rename, Resolution… (a W/H form), Make primary, Pop out and Remove (disabled for the last monitor); `+` opens a preset picker with `Custom…` and Right of / Below. Props in (`monitors`, `active`, `presets`), callbacks out (`onSelect`, `onAdd`, `onRename`, `onResize`, `onPrimary`, `onPopOut`, `onRemove`). Styles: `styles/display.css` (U10b) |
| `DisplayArrange.jsx` | the arrangement in an OS display dialog: one draggable rectangle per monitor in SVG, snapping to the nearest free slot beside a neighbour while the pointer moves; the mode as two buttons (`Separate screens` / `One stretched screen`); the selected monitor's preset, W/H and `⟲ Rotate`; the live counters (`3200×1080 · 2/8 monitors · 3.5 MP of 36 MP`, red when a limit is hit) and the status line. It takes the layout maths from the pure `features/sandboxes/displayLayout.js` and never decides what is allowed; the layout is the caller's. Styles: `styles/display.css` (U10c) |


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `BulkBar.jsx` | The bar that appears above the dock while rows are selected (contract C15, package F21). |
| `HostLoad.jsx` | The host's own load, live: CPU, memory, disk, load average, and how much the |
| `LogView.jsx` | A log, read the way an operator reads one: newest at the bottom, problems in colour, |
| `LogView.test.js` | Node tests for `LogView`. |
| `Meter.jsx` | A labelled horizontal bar. `value` and `max` are numbers in the same unit; |
| `Poll.js` | Poll a loader while a component is mounted. `load` returns a promise; each answer goes to `onData`, a failure to `onError`. A hidden tab (`document.hidden`) stops the timer without touching the pause switch; on becoming visible one `tick()` runs at once and the interval resumes, and the `visibilitychange` listener is removed in `onCleanup`. `pollActive({ hidden, paused })` is the pure decision, so the behaviour is testable without a DOM. |
| `poll.test.js` | Node tests for `pollActive`: hidden always quiet, paused quiet, visible and unpaused polls, coming back into view runs one tick; and the module keeps the listener removal. |
| `TabBoundary.jsx` | A tab must never take the console down with it. |
| `alertAnswers.test.js` | Node tests for `alertAnswers`. |
| `countdown.test.js` | Node tests for `countdown`. |
| `markdown.test.js` | Node tests for `markdown`. |
