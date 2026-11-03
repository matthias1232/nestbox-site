# features/ — one folder per screen

## Purpose

Each folder is one part of the console: its page, its pieces, and its logic.
A feature may import from `api/`, `components/`, `hooks/`, and `state/`; it may
not import from another feature.

That rule is what keeps a change local. If two features need the same thing, it
belongs in `components/` or `api/`, not in a cross-import.

## Layout of a feature

```
features/sandboxes/
├── AGENTS.md            ← written before the first component
├── SandboxListPage.jsx  ← presentation
├── SandboxDetail.jsx    ← presentation
└── useSandboxes.js      ← state, paging, calls; no JSX
```

## Rules

1. `use*.js` holds the logic and contains no JSX, so it is testable without a
   DOM.
2. A page fetches through its hook, never directly in JSX.
3. A feature renders its own errors; a silent failure is a bug.
4. Empty and loading states are explicit, because "nothing" and "not yet" look
   the same otherwise.

## Files

| Folder | Screen |
|---|---|
| `auth/` | Login |
| `sandboxes/` | Sandbox list and detail drawer |
| `traffic/` | Live traffic console |
| `policies/` | Policies, rules, drafts |
| `cluster/` | Gateways, supervisors, topology |
| `providers/` | Model endpoints and their credential locators |
| `models/` | Model catalog with hardware fit |
| `jobs/` | Jobs and schedules |
| `nodes/` | The machines sandboxes and models run on, their budget, and the probe |
| `recipes/` | Recipe table, editor, detail, build, instantiate |
| `persistent/` | Persistent roots, their mounts, and usage |
| `media/` | Media rendering models: the environment default, the allow-list and a sandbox's own choice |
| `builder/` | Composing a recipe from the catalogs, then building and running it |

## Extending

Create the folder, write its `AGENTS.md` first, then the hook, then the page.
Add a row to this table and a route in `App.jsx`.

## Multi-tenant and multi-node screens

| Folder | Screen |
|---|---|
| `tenants/` | Data centers and sites, customers, the context switcher |
| `images/` | Image registry and sync: the matrix and the assignments |
| `moves/` | Move a sandbox to another node and back |
| `hosting/` | Provider accounts and rented machines |
| `users/AccessSection.jsx` | Share a sandbox with one person; invitations |
| `nodes/JoinNode.jsx` | Add a node |

| `workbox/` | the end-user view of one sandbox and the list of my sandboxes |

`workbox` may import components from `features/sandboxes/` (`TerminalTab`, `DesktopTab`,
`StackTopology`, `chats.js`): an explicit exception, because a second terminal and a second desktop
would be a second truth.
