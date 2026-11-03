# web-console-solid — the operator console

## Purpose

A SolidJS console for the Nestbox control API. It is a client, not an
authority: every rule is enforced by the server, and the console only decides
what to show and what to offer.

## Layout

| Path | What |
|---|---|
| `src/api/` | One file per backend resource; the only place that fetches |
| `src/components/` | Reusable UI with no domain knowledge |
| `src/features/` | One folder per screen |
| `src/state/` | Session and environment |
| `src/styles/` | Tokens and layout |
| `nginx.conf` | Same-origin proxy to the API; gzip for text assets, never for `/api/` (SSE) |

## Build and run

```bash
npm ci
npm run build      # writes dist/
npm run dev        # Vite dev server, proxies /api via the same origin in prod
```

In the appliance the console is built into an image and served by nginx on
`:8080`, with `/api/` proxied to the control API. There is no CORS configuration
because there is one origin.

## Rules

1. No `fetch` outside `src/api/client.js`.
2. The CSRF token and any credential live in memory, never in `localStorage`.
3. Loading, empty, and error states are explicit.
4. A destructive action asks once and says what it destroys.
5. Read `docs/adr/` before changing how a rule, a policy, or a model is
   presented; several UI decisions exist to avoid claiming something the runtime
   does not do.

## Known limits

- Routing is a signal, not a router: a reload returns to the sandbox list.
- After a reload the CSRF token is gone, so a write requires signing in again.
  Read-only views keep working, which is the intended trade.

## Bundle layout

Pages (`features/shell/pages.js`, the route imports in `App.jsx`) and the terminal/desktop tabs
(`TerminalTab`, `DesktopTab`, imported at `AdminCenter`, `ConsoleFrame` and the pop-outs) are
`lazy()` imports. `vite.config.js` splits `@xterm/*`, `@novnc/novnc` and `solid-js` into their own
chunks, so xterm and noVNC load only when a console opens. Keep Login, Shell and the session state
eager. New pages must use `lazy(() => import(...))`.
