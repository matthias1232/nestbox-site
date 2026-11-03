# features/shell/ — the frame around every page (F01)

The Shell composes the topbar, the sidebar (tree and menu), the content area and the dock, and
mounts the overlays once. `App.jsx` wraps every page except the pop-outs in `<Shell>`.

| File | Responsibility |
|---|---|
| `Shell.jsx` | Grid of topbar, sidebar, content, dock; mounts `EXTRAS` once |
| `Topbar.jsx` | Wordmark, data-center picker, search trigger, alert bell, help, user menu |
| `Sidebar.jsx` | Tree views, filter and `Tree` above collapsible menu groups from `menuFor(role)`; resizable, collapsible to icons |
| `Dock.jsx` | Tasks / Live events / Alerts tabs, collapsed by default, resizable, pop-out, bottom sheet under 720 px |
| `Breadcrumb.jsx` | The trail at the top of a page |
| `PageHeader.jsx` | Sticky title row with status and the action bar |
| `useInventory.js` | Shared inventory tree hook: one 10 s poll, live-event refresh (debounced 500 ms), view from `ui.tree` |
| `treeFilter.js` | Pure helpers: `filterTree`, `treePathFor` (breadcrumbs), `pathFor`, `ancestorIds`, `expandableIds` |
| `treeFilter.test.js` | Tests for the helpers above |
| `pages.js` | `PAGES` (menu id to page) and `EXTRAS` (overlays). Owned by Wave 0: packages never edit it |
| `PaletteHost.jsx` | Command palette host (F04) |
| `BulkHost.jsx` | Bulk selection bar host (F21) |

`data-tour` markers: `tree`, `context`, `search`, `actions`, `dock` (the first-run tour, F18).
