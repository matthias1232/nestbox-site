# persistent/ — the data that outlives a container

## Purpose

Persistent roots are directories on a node; a mount binds one into a sandbox.
This screen lists the roots, creates one, mounts and dismounts it, and archives
or deletes the record. ADR 0021 is the rulebook behind every sentence here.

## Files

| File | Responsibility |
|---|---|
| `PersistentPage.jsx` | The table (kind, node, slug, host path, used, mounted in) and the create form |
| `PersistentDetail.jsx` | The drawer: facts, mounts, the mount form, archive and delete |
| `usePersistent.js` | List, selection, usage, the sandbox list for the mount form, actions |

Styles live in `src/styles/recipes-persist.css`, imported by the page.

## Decisions that must not be quietly undone

1. **The host path is never an input.** The server derives it from kind, owner,
   and slug. A path field would be a way to ask for arbitrary host access, which
   is exactly what the API refuses to offer.
2. **The mount path is typed, not picked.** The control plane cannot list a
   node's filesystem, so a picker would promise a browse that does not exist. The
   server validates the path, and its refusal is shown as it came.
3. **A 409 is shown with the server's message and its code.** `mount.conflict`
   names the sandbox that holds the read-write mount; `root.in_use` names the
   sandboxes that still mount the root. The console does not translate either
   into a friendlier guess, because the name is the actionable part.
4. **"not measured" is not zero.** A root without `measured_at` shows
   `not measured`, never `0 B`. The control plane cannot `du` a remote node's
   disk, and a fabricated number is worse than an empty field.
5. **Archive and delete say what they do to the data.** Archiving hides the root
   and keeps the directory; deleting the record leaves the directory on the node.
   Neither is offered as "delete the data", because neither deletes files.
6. **An archived root leaves the list.** `GET /persistent-roots` excludes
   archived rows and there is no restore endpoint; the page says so instead of
   implying a filter that would bring one back.

## Wiring

`App.jsx` is not part of this package: the integrator adds the route and the
`features/AGENTS.md` row. The page exports a default component with no props.

Restyle (F25): `PersistentPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `BackupsSection.jsx` | Backups of one persistent root: take one, see the list, restore, delete. |
| `MountsMatrix.jsx` | Folders × sandboxes: the whole map at a glance. |
