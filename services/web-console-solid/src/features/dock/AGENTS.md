# features/dock/ — the bottom dock (F13)

| File | Responsibility |
|---|---|
| `TasksTab.jsx` | Jobs with progress meter, status badge, last log line, Open (F13, F26 adds labels); hides jobs whose payload carries `source: "acceptance"` (an acceptance run is not the operator's work) |
| `LiveEventsTab.jsx` | The live event ticker: 200 lines, newest top, pause, tone filter |
| `AlertsTab.jsx` | Open refused destinations as rows with inline answers; ignore all |
| `AlertStack.jsx` | Alert pop-ups top-right (max 3), mounted once by the Shell; starts the alert polling; imports `styles/dock.css` |
| `useAlerts.js` | Shared alert state: 15 s poll + live `alert` merge, `answer()`, bell count |
| `eventLines.js` | Pure `lineFor(name, payload)`, `pushLine`, `filterTone` for the ticker (tested in `eventLines.test.js`) |
