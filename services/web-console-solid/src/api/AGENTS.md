# api/ — how the console talks to the server

## Purpose

One file per backend resource. Each file knows the URL, the method, and the
request and response shape. Nothing here knows about a screen, a component, or a
signal.

This layer exists so a change to an endpoint is one edit in one place, and so a
component never contains a URL.

## Rules

1. No `fetch` outside `client.js`. The client adds the CSRF header, the
   environment header, and the error mapping once.
2. A function returns parsed data or throws `ApiError`. It never returns a
   response object, because that would leak HTTP details into a component.
3. No state. These are functions, not hooks; state lives in `features/*/use*.js`
   or `state/`.
4. A path is written once. If two features need the same call, they import the
   same function.

## Files

| File | Resource |
|---|---|
| `client.js` | fetch wrapper, CSRF, environment header, error mapping with `code` and `details` |
| `auth.js` | login, logout, identity, environments |
| `sandboxes.js` | list, detail, create, actions, logs, exec, inference, rules, agents |
| `nodes.js` | nodes with their computed budget: free memory, cores, sandbox count |
| `packs.js` | the agent-pack catalog, optionally narrowed to a baseline |
| `policies.js` | policies, rules, drafts, traffic windows |
| `telemetry.js` | history, facets, the SSE stream |
| `traffic.js` | the environment-wide traffic list, with counts |
| `cluster.js` | gateways, settings, sync, supervisors, topology |
| `providers.js` | profiles, probe; locators only, never a value |
| `models.js` | catalog with filters, hardware, facets, requirements, installs, install, uninstall |
| `nodes.js` | nodes with their budget, one node, its sandboxes, probe, display rename |
| `packs.js` | baselines, agent packs, and the tool catalog, resolved for a baseline |
| `recipes.js` | recipe CRUD, build, instantiate |
| `persistent.js` | persistent roots, mount, dismount, usage |
| `jobs.js` | jobs, logs, schedules |

## Extending

Add the function to the file of its resource. If the resource is new, add a file
and a row to the table above.

Workbox modules: `control.js` (one summary per sandbox, one overview), `agentAuth.js`,
`presence.js`, `modelGrants.js`, `nodesConnect.js`; `moves.js` gained `preflight` and
`policies.js` gained `bulkDrafts` and the revert timer.

Control Center modules (Wave 0): `inventory.js` (tree), `search.js`, `bulk.js` (sandbox bulk job),
`clusters.js` (model clusters), `nodeOps.js` (summary, drain, quick deploy), `alerts.js`, `mounts.js`,
`quick.js` (one-click controls), `permissions.js`, `tours.js`, `events.js` (opens an SSE stream for the
shared live store), `help.js` (static help files; the one module besides `client.js` that calls `fetch`).
`models.js` gained `installBulk`, `policies.js` gained `preview` and `compiled`.
