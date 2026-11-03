# src/ — the console

## Purpose

A SolidJS console for the Nestbox control API. It reads and writes through
`api/`, holds screen logic in `features/*/use*.js`, and renders with components
that know nothing about the domain.

## Layers

| Folder | Holds | Must not contain |
|---|---|---|
| `api/` | One file per backend resource | components, state |
| `components/` | Reusable UI | fetching, domain rules |
| `features/` | One folder per screen | cross-feature imports |
| `state/` | Session and environment | screen logic |
| `styles/` | Design tokens and layout | component-specific hacks |

## The menu

`navigation.js` is the menu as data: six groups (Overview, Operate, Setup,
Security, Administration, Help) and the route list with a minimum role per entry.
`App.jsx` only renders it. A new screen is one line there. `navigation.test.js`
(`node --test src/navigation.test.js`) checks that every route is in exactly one
group and that a role never sees an entry above it. Hiding is convenience; the
API enforces the same rule.

| Row | Meaning |
|---|---|
| `minRole` | The entry is hidden from a role below it; the API refuses such a call. |
| `hidden: true` | The entry stays out of the menu (sidebar, palette, help search) while the page and its path stay, for a page whose API is not there yet — `live` today. |

## Rules

1. No `fetch` outside `api/client.js`.
2. No URL string in a component; call an `api/` function.
3. A screen's logic lives in its `use*.js` so it can be tested without a DOM.
4. Loading, empty, and error states are explicit — otherwise "nothing" and "not
   yet" look identical, which is how a broken screen looks healthy.
5. The CSRF token lives in memory only. `localStorage` is not used for
   credentials, because anything in storage is readable by an injected script.
6. Every string a reader sees is English. There is no i18n framework, because
   the console ships one language; `noGerman.test.js` scans `src/**/*.{js,jsx}`
   for German characters and a short German word list and fails on them
   (comments and test files are out of scope).

## Adding a screen

1. Create `features/<name>/` and write its `AGENTS.md`.
2. Write `use<Name>.js` with the state and the calls.
3. Write `<Name>Page.jsx` with markup only.
4. Add the route in `App.jsx` and a row to the table in `features/AGENTS.md`.
   The Admin Center is the one view with a real address (`/sandboxes/<id>`), so
   a link to a sandbox survives a reload and can be pasted into a chat; every
   other view stays a signal.

`routing.js` says what a URL means (Admin Center, workbox, pop-outs); `App.jsx` only renders it.
A pop-out window shares the opener's CSRF token (`adoptOpenerCsrf`) instead of rotating it.

Control Center routing (Wave 0): `routing.js` implements contract C1 (`paths`, `parsePath`, `SHELLLESS`); the address
bar is the only source of truth. `navigation.js` holds the six menu groups; `App.jsx` maps `parsePath` results to pages
and wraps them in `features/shell/Shell.jsx`.
