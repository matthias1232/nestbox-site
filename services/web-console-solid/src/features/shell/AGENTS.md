# features/shell/ — the frame around every page (F01, Wave 0 skeleton)

The Shell composes the topbar, the sidebar (tree and menu), the content area and the dock, and
mounts the overlays once. `App.jsx` wraps every page except the pop-outs in `<Shell>`.

| File | Responsibility |
|---|---|
| `Shell.jsx` | Grid of topbar, sidebar, content, dock; mounts `EXTRAS` once |
| `Topbar.jsx` | Wordmark, data-center picker, search trigger, alert bell, help, user (F01 completes) |
| `Sidebar.jsx` | Inventory tree (F01) above the menu groups from `menuFor(role)` |
| `Dock.jsx` | Tasks / Live events / Alerts tabs, collapsed by default |
| `Breadcrumb.jsx` | The trail at the top of a page |
| `PageHeader.jsx` | Sticky title row with status and the action bar |
| `useInventory.js` | Stub of the inventory tree hook (F01 fills; F05/F06/F17 use it) |
| `pages.js` | `PAGES` (menu id to page) and `EXTRAS` (overlays). Owned by Wave 0: packages never edit it |
| `PaletteHost.jsx` | Command palette host (F04) |
| `BulkHost.jsx` | Bulk selection bar host (F21) |

`data-tour` markers: `tree`, `context`, `search`, `actions`, `dock` (the first-run tour, F18).
