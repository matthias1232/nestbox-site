# features/dock/ — the bottom dock (F13)

| File | Responsibility |
|---|---|
| `TasksTab.jsx` | Jobs with progress meter, status badge, last log line, Open (F13, F26 adds labels); hides jobs whose payload carries `source: "acceptance"` (an acceptance run is not the operator's work) |
| `LiveEventsTab.jsx` | The live event ticker: 200 lines, newest top, pause, tone filter |
| `AlertsTab.jsx` | Open refused destinations as rows with inline answers; ignore all. Its head line names the pop-up state (`Pop-ups: on` / `muted until 15:40` / `muted`) with the mute toggle, and the list always shows every open alert, muted or not (U05) |
| `AlertStack.jsx` | Alert pop-ups top-right (max 3), mounted once by the Shell; starts the alert polling; imports `styles/dock.css`. Renders nothing while the pop-ups are muted, skips cards the operator hid, hides every card after 12 s, and the card's × calls `hide`; the count is `open().length` and stays untouched (U05) |
| `useAlerts.js` | Shared alert state: 15 s poll + live `alert` merge, `answer()`, bell count |
| `eventLines.js` | Pure `lineFor(name, payload)`, `pushLine`, `filterTone` for the ticker (tested in `eventLines.test.js`) |
