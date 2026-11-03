# state/ — shared stores

| File | Responsibility |
|---|---|
| `session.jsx` | Identity, CSRF token (memory only), active data center; remembers the chosen data center per user in `localStorage['nestbox:environment:<user_id>']` (an id, never a token) and opens it on sign-in and after a reload while the server still lists it; the login answer carries no user id, so sign-in asks `auth/me` once for the key (X15) |
| `environments.js` | Pure: `pickEnvironment` (remembered choice, then the server's current one), `storageKey`, and `emptyDataCenterNotice` for a data center without nodes |
| `environments.test.js` | Node tests for `environments`. |
| `ui.jsx` | UI preferences (persisted under `nestbox:ui`), selection, toasts, palette, help, alerts, `onKey` |
| `uiPrefs.js` | Pure: defaults, sanitising, serialising of the persisted preferences |
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
| `uiPrefs.test.js` | Node tests for `uiPrefs`. |
