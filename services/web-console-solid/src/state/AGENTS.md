# state/ — shared stores

| File | Responsibility |
|---|---|
| `session.jsx` | Identity, CSRF token (memory only), active data center; remembers the chosen data center per user in `localStorage['nestbox:environment:<user_id>']` (an id, never a token) and opens it on sign-in and after a reload while the server still lists it; the login answer carries no user id, so sign-in asks `auth/me` once for the key (X15); with no remembered choice it opens the first data center that has nodes (`firstUsable`, counts from `GET /api/v1/organizations`), switches with `{ remember: false }` and publishes `autoSwitched` so the sidebar can say so once (U02) |
| `environments.js` | Pure: `pickEnvironment` (remembered choice, then the server's current one), `storageKey`, `emptyDataCenterNotice` for a data center without nodes, `environmentsWithNodes` (flattens `GET /api/v1/organizations` to `{ id, name, nodes }`, because `/environments` carries no node count) and `firstUsable` (the current data center while it has nodes, else the first one that has, else null) (U02) |
| `environments.test.js` | Node tests for `environments`. |
| `ui.jsx` | UI preferences (persisted under `nestbox:ui`), selection, toasts, palette, help, alerts, `onKey`, and `console` (`mode(id)`, `setMode`, `vncOptions(id)`, `setVncOptions(id, patch)` for the desktop viewer). `alerts` carries the bell count, `ignore`/`isIgnored` (forget an alert for the session), the pop-up mute (`popups()`, `popupEnds()`, `setPopups(value)`, persisted, woken by a timer when a timed mute ends) and `hide(id)`/`isHidden(id)` (hide one pop-up card, memory only). Neither the mute nor hiding a card touches the count |
| `uiPrefs.js` | Pure: defaults, sanitising, serialising of the persisted preferences, including `alertPopups` (`'on'`, `'off'`, `{ until }`) and `vncOptions` (the desktop viewer's noVNC options per sandbox, completed and clamped by `features/sandboxes/vncOptions.js`) (U04) |
| `alertMute.js` | Pure: `resolvePopups(value, now)` turns a stored pop-up choice into `'on'`/`'off'` (an expired timed mute is `'on'`), `popupsUntil(value, now)` names the end of a live timed mute (U05) |
| `shortcuts.js` | Pure: parsing and matching `mod+k` and `g h` |
| `live.jsx` | The app's live streams: one state `EventSource` per window (a pop-out only while one of its pages listens), one telemetry `EventSource` while some screen holds `wantTelemetry()`, both reference counted; `on(name, handler)` returns the stop function; reconnect with backoff; `status()` |
| `liveCore.js` | Pure: the C18 event names (`STATE_EVENTS`, `TELEMETRY_EVENTS`), the handler registry, `createRefCount` (a shared stream closes only when the last request leaves), the reconnect delay, the alert payload |

Nothing here stores a token or a secret in `localStorage`.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `liveCore.test.js` | Node tests for `liveCore`. |
| `streams.test.js` (in `src/`) | Node tests for the stream budget: reference counting and the reconnect delay, the C18 names, and that no file outside `api/events.js`, `api/chats.js` and `api/live.js` names an `EventSource`. |
| `environments.test.js` | Node tests for `environments`. |
| `shortcuts.test.js` | Node tests for `shortcuts`. |
| `uiPrefs.test.js` | Node tests for `uiPrefs`, including the three pop-up shapes and their fallbacks, and the per-sandbox desktop options. |
| `alertMute.test.js` | Node tests for `alertMute`: default, `'off'`, a timed mute before and at its moment. |
