# features/moves

Moving a sandbox, with its persistent folders and the agent's login, from one node to another and
back (ADR 0049). `MovesPage.jsx` has a form (sandbox, target node) and the history, each move with
the steps it took. A move runs in the background on the control plane; this page polls it.

The source keeps its files as a cold copy. Nothing here deletes anything.

Restyle (F25): `MovesPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
`MovesPage.jsx` now reads as a task list: a table with a nine-step strip per move, a drawer with
steps, log and snapshots, "Move a sandbox…" (picker, then the workbox `MoveDialog`) and "Recover…".
Styles are in `styles/jobs.css`.
