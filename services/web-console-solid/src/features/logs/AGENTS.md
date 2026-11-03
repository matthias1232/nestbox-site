# features/logs/ — live logs

| File | Responsibility |
|---|---|
| `LogsPage.jsx` | The live logs page in the shell; also exports `LogsBoard` (panes + red lane) and `addPane` |
| `LogsPopout.jsx` | `/pop/logs/<id or all>` in its own window, no shell; pane layout is local to the window |
| `LogPane.jsx` | One pane: source select, follow, filter, problems only, copy, pop-out, close |
| `RedLane.jsx` | Sticky strip of refused destinations of the shown sources, with the inline alert answers |
| `logLoaders.js` | Pure source helpers: `loaderFor`, `mergeByTime`, `stamp`, `sourceLabel`, `sandboxIdsOf` |
| `logLoaders.test.js` | Tests for the pure helpers |

Pane sources are strings (`all`, `sandbox:<id>`, `node:<id>`, `system:<name>`) saved in `ui.logs.panes()`.
Styles: `styles/logs.css`.
