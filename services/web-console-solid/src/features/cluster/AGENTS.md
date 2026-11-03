# cluster/ — gateways, supervisors, and the graph

## Purpose

Show where workloads run and whether the control plane and the gateway agree.

## What it must make obvious

- **Drift**: the gateway's desired settings version differs from the applied one.
  That is the state the sync button exists for, so it has to be visible without
  opening anything.
- **Placement safety**: a drifted or unhealthy gateway receives no new sandboxes.
- **The flow**: gateway → sandbox, so "which gateway carries this workload" is
  answerable by looking.
- **What the supervisor measured**: its best-practice checks are shown per row
  with the state and the measured value in the tooltip, and the two actions an
  operator repeats (check, restart the agent) are one click. The whitelist lives
  in the supervisor, so a button here cannot invent a third action.

## The traffic layer on the graph

The graph is placement first. A traffic window (`no traffic` / `5m` / `15m` /
`1h`) adds what actually left each sandbox: one node per destination, and an edge
whose thickness is the request count. The window is a choice, not a default
truth — the static graph is what four other screens read.

A destination that had requests **refused** is the one thing here an operator
must act on, so it is drawn red and solid while every other traffic line is
faint. Clicking it offers the same bounded answers as the decisions view: allow
for 15/30/60 minutes on the destination's own channel. It never creates a rule —
a permanent answer is a draft under Policies (ADR 0011).

## Files

| File | Responsibility |
|---|---|
| `ClusterPage.jsx` | Gateways, sync, supervisors, topology drawn by `TopologyCanvas` (traffic window switch, side panel with Allow 15/30/60 on a refused destination) |
