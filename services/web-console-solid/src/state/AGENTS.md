# state/ — shared stores

| File | Responsibility |
|---|---|
| `session.jsx` | Identity, CSRF token (memory only), active data center |
| `ui.jsx` | UI preferences (persisted under `nestbox:ui`), selection, toasts, palette, help, alerts, `onKey` |
| `uiPrefs.js` | Pure: defaults, sanitising, serialising of the persisted preferences |
| `shortcuts.js` | Pure: parsing and matching `mod+k` and `g h` |
| `live.jsx` | One shared `EventSource` for the app, reconnect with backoff, `wantTelemetry` |
| `liveCore.js` | Pure: handler registry, reconnect delay, alert payload |

Nothing here stores a token or a secret in `localStorage`.
