# ADR 0059 — The console is an inventory tree, and every view has a real address

Status: Accepted
Date: 2026-11-02

Note: the number 0059 is also used by `0059-content-inspection-on-the-decrypted-hop.md`
(and 0060–0062 by other files). The Control Center plan fixed these numbers before the
clash was seen; the file names are unique. Renumbering is left to the maintainer.

## Context

The console grew one page per feature behind a flat menu. An operator with twenty nodes
had to know which menu entry held the node, the sandbox, or the model, and a link to
"that sandbox" did not exist: reloads and pop-outs lost where you were. The platform
itself is a tree (ADR 0044): organisation › data center › site › node › sandbox. The
console should show the thing the platform is, the way vCenter or Proxmox do, and a
member who owns one sandbox must still see a small, honest version of the same thing.

The console is also meant to be small and quick on weak machines: SolidJS, plain CSS,
inline SVG, no router library, no state library.

## Decision

* **The sidebar is a tree** of organisation › data center › site › node › sandbox with
  three views: *Infrastructure* (the hierarchy above), *Workloads* (data center › user ›
  sandbox) and *Models* (model › install; model clusters beside them). A view is a
  filter on the same data, not a separate page.
* **Every view has a real path** (`/dc/<id>`, `/site/<id>`, `/node/<id>`,
  `/sandboxes/<id>?tab=`, `/box/<id>`, `/models/<key>`, `/clusters/<id>`, `/docs/<slug>`,
  `/pop/logs/<id>`, `/pop/traffic`). Links, reloads, the browser's back button and
  pop-out windows all work because the address carries the place. `parsePath` and the
  `paths` helpers in one file are the only place that knows the shapes.
* **The menu is secondary.** It holds what the tree cannot: policies, traffic, setup
  pages. Entries are grouped, collapsible and filtered by role.
* **One tree endpoint feeds the shell.** `GET /inventory/tree?view=` returns the whole
  tree with counts and status per level, so the shell polls one answer (and refreshes on
  live events) instead of five lists. A node with more than 400 sandboxes is truncated and
  says so.
* **The dock holds what happens now**: Tasks (jobs), Live events and Alerts, each with a
  pop-out. It is the one place where long-running work is visible.
* **No router library and no state library.** Routing is a pure function over
  `location`, state is a handful of Solid signals in three stores (session, ui, live).
  The reason is size and memory: both were measured as the cost the console cannot afford
  on a small appliance browser, and neither buys anything a 100-line module does not.
* **What a member sees.** The tree shows their data center, their site and node (names
  only, no budget) and their own or granted sandboxes; Row-Level Security does the
  filtering, the service opens one scope per environment. The menu shows My sandboxes,
  Live traffic and Live logs (own), and Docs; the Workbox (`/my`) is their home. Hiding is
  a convenience; the API refuses.

## Alternatives rejected

* **A router library.** It adds weight and a second source of truth for the place; the
  path table is small and testable.
* **One list endpoint per level.** Five requests per refresh, five loading states, and
  counts that disagree with each other.
* **Keeping the flat menu and adding a search.** Search finds a thing you can name; the
  tree shows what exists. Both are built, neither replaces the other.
* **A separate member console.** Two codebases drift. One console with role-filtered data
  is cheaper to keep right.

## Consequences

Adding a kind of object means: a tree node kind, a path, a page, and a row in the
navigation tables. The shell never loads a full list of sandboxes beyond the 400-row cap
per node. A path is part of the public contract of the console; renaming one breaks
saved links.

## Not built

Drag-and-drop moves in the tree, user-defined folders, and saved tree filters.

## Invalidation conditions

If the tree grows past what one response can carry (many thousands of sandboxes per data
center), it becomes lazy per level with the same node shape.
