# features/shell/ — the frame around every page (F01)

The Shell composes the topbar, the sidebar (tree and menu), the content area and the dock, and
mounts the overlays once. `App.jsx` wraps every page except the pop-outs in `<Shell>`.

| File | Responsibility |
|---|---|
| `Shell.jsx` | Grid of topbar, sidebar, content, dock; mounts `EXTRAS` once |
| `Topbar.jsx` | Wordmark, data-center picker, search trigger (shortcut as `KeyHint keys="mod+k"`, X10), alert bell with its count badge, help, user menu. The bell and the help button draw `Icon name="Bell"` / `Icon name="Help"` from `components/icons.jsx`, never a text glyph (V02) |
| `Sidebar.jsx` | Tree views, filter and `Tree` above collapsible menu groups from `menuFor(role)`; resizable, collapsible to icons: a local `MENU_ICONS` map picks an `Icon` per route, `Settings` for an unknown one (X10); an empty data center gets a one-line notice with a switch button instead of a silent switch (X15); a node's allow-all state draws as a lime `ALL` badge, refused destinations as a red word badge (V05); the view tabs sit in `.cc-view-tabs`, one fixed-height row inside `.cc-tree-controls`, with the tree following in normal flow below them (V13); the menu groups start closed except `overview` and `operate` (`closedGroups` in memory only) and the nav scrolls inside its own capped box, so the tree keeps at least ~45 % of the sidebar height (V17); the tree opens the organisation and every data center once it loads (`defaultExpanded`, skipped after the first user toggle via `touched` and while a filter is active) (V18) |
| `Dock.jsx` | Tasks / Live events / Alerts tabs, collapsed by default, resizable, pop-out, bottom sheet under 720 px. The bar itself names the newest running task (muted mono, `labelFor` + `summaryLine`, own 15 s poll of `api/jobs.js`) and, right-aligned in `--bad`, the newest unanswered refusal as `● sandbox → host:port refused` from `useAlerts().open()[0]` (V06) |
| `Breadcrumb.jsx` | The trail at the top of a page |
| `PageHeader.jsx` | Sticky title row with status and the action bar. At >= 1100 px the title block and the action block sit in one grid row (title flexible, actions a separate right column) and actions wrap inside their own column, never over the title; below 1100 px the two blocks stack. Exactly one action is `primary` (V07) |
| `useInventory.js` | Shared inventory tree hook: one 10 s poll, live-event refresh (debounced 500 ms), view from `ui.tree`. A load is skipped while the last good answer for the same view is younger than 2 s (`FRESH_MS`), so the sidebar, home and data-center pages that mount together cost one request; `refresh()` passes `force` and always loads |
| `treeFilter.js` | Pure helpers: `filterTree`, `treePathFor` (breadcrumbs), `pathFor`, `ancestorIds`, `expandableIds`, `defaultExpanded` (the root plus every data center, the sidebar's opening state, V18) |
| `treeFilter.test.js` | Tests for the helpers above |
| `placeEnvironment.js` | Pure helper: the id of the data center that holds a place (`dc`, `site`, `node`, `admin`, `box`, `model`), or null; lets a link to another data center's entity move the session once instead of answering "does not exist" (V15) |
| `placeEnvironment.test.js` | Tests for the helper above |
| `pages.js` | `PAGES` (menu id to page) and `EXTRAS` (overlays). Owned by Wave 0: packages never edit it |
| `PaletteHost.jsx` | Command palette host (F04) |
| `BulkHost.jsx` | Binds `ui.selection` to `BulkBar`, asks for the input an action needs, calls `api/bulk.js` or `installBulk`, opens the dock Tasks tab (F21). Style: `styles/bulk.css` |

`data-tour` markers: `tree`, `context`, `search`, `actions`, `dock` (the first-run tour, F18).

F04: `PaletteHost.jsx` registers `mod+k`, loads the help index on first open, feeds `CommandPalette` (search, menu, docs, recents, data centers) and navigates on Enter. Style: `styles/palette.css`.
