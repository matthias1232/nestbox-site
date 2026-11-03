# Scope of the current phase (M1)

This document is deliberately explicit about what M1 does **not** do. An honest
limit is more useful than a silent gap, because a limit can be planned around
and a gap cannot.

## Built in M1

| Area | What is built |
|---|---|
| Control store | PostgreSQL, migrations, row-level security, seed environment `default` |
| API | Typed domain, layered architecture, session auth, RBAC, tenant scope, cursor paging, facets, SSE |
| Console | Sandbox list with server paging and bulk jobs, live traffic console (network, DNS, inference), policy editor with diff and rollback, rule drafts, gateways with drift and sync, models with live fit, jobs and schedules, audit |
| Policy | Four-section model, two scopes, templates, rollout, approval workflow, temporary traffic windows |
| Telemetry | Three live streams, partitioned history, retention by partition drop |
| Models | Catalog as data, hardware detection, fit classification, install and update jobs, local and cloud |
| Ingress | Rules, certificates inventory, address pools, forwards, service exposures — data model and API |
| Operations | Jobs, schedules, audit, live logs per component |

## Modelled but not executed in M1

These have a complete schema and API surface. The execution path is M2.

| Capability | Why not yet |
|---|---|
| Multi-gateway overlay routing | Needs a second real gateway to test against |
| Ingress renderer (generate nginx config per gateway and reload atomically) | Needs a real gateway to validate the generated config |
| ACME certificate issuance and renewal | Needs a public domain or a PKI decision |
| Internal CA issuance | Same |
| Sandbox migration between gateways | The data model is ready; the live move needs two gateways |
| Transparent proxy interception | The mode and model exist; issuing the trust CA is M2 |
| mTLS between gateway and control plane | Needs a certificate authority, which is M2 |

## Not in scope at all

| Capability | Reason |
|---|---|
| Kubernetes or Helm deployment | The runner and bind-mount persistence are the contract (ADR 0021, 0047) |
| Multi-region control planes | Postgres is the single store; regions would need replication. Multi-site is different and is in scope: one control plane, many sites and nodes (ADR 0044, 0047) |
| A marketplace of third-party packs | Out of the current product statement |
| Billing or invoicing | Usage is recorded (ADR 0050 budget guards); invoices are not part of the product |

## Known environment limits

The development machine has no GPU. That means:

- Hardware detection is verifiable and correctly reports no GPU.
- The model fit classification is fully verifiable.
- The download and server-start path cannot be exercised end to end on that
  machine. It is built in full, and verified against the classification, the
  job stages, and the error paths.

On a machine with a GPU, the same code runs unchanged. This is stated rather
than hidden, because a claim of "tested" that excludes the one path that needs
hardware would not be honest.

## What M1 proves

The structure. If the sandbox list arrives with server paging, facets,
virtualisation, and bulk jobs without modifying an unrelated module, the layering
holds and every later feature is cheaper. If it required touching everything, the
structure was wrong and was corrected before continuing.

## Multi-tenant and multi-node (planned, ADR 0044-0050)

Organisation tree, data centers and sites, role bindings down to one sandbox,
OpenID Connect sign-in, nodes that join over a WireGuard overlay, a synchronised
image registry, snapshot-based migration and provider drivers. Status per phase
is tracked in the project plan and in `phase1-review.md` once a phase is proven
on the appliance. Until a phase says otherwise there, it is a decision, not a
feature.

### Status of the multi-tenant and multi-node work (2026-11-01)

Proven on the development appliance (192.168.55.50) with a second VM and a Proxmox host:
organisation tree, data centers, sites, member role and sandbox grants, invitations, node join over
WireGuard, per-node runner dispatch, image registry sync, sandbox move there and back, rented
machines from Proxmox with deletion. Proven by tests only (no live provider): OIDC sign-in flow
(profile parsing, registration, invitation, replay), Hetzner driver request shapes.

Also proven since: scheduled snapshots of folders working on a joined node, pruning, and recovery of a
sandbox after its machine was powered off. DigitalOcean and Vultr drivers: unit tests only.

Not built, deliberately listed: Brev/AWS/GCP drivers; models, builds and CUDA-X on a joined node;
node-to-node links; automatic (unattended) failover; site-level roles and a general `role_bindings`
table; billing. See the amendments of ADR 0044-0050.

## Control Center (2026-11)

### Built

- C1 Console paths: data center, site, node, model, cluster and docs pages, plus pop-out logs and traffic.
- C2 Shell: tree sidebar, top bar, dock, palette, alert stack, help drawer, shortcuts sheet and tour.
- C3 Inventory tree with three views (infrastructure, workloads, models).
- C4 Search across nodes, sandboxes, models and policies.
- C5 Bulk sandbox actions in one request.
- C6 Bulk model deploy across many nodes as one job.
- C7 Model clusters: installs of one model behind one provider profile, round-robin.
- C8 Node operations: summary, drain, quick deploy.
- C9 Alerts, with acknowledgement and live delivery.
- C10 Effective mounts for a sandbox.
- C11 Policy preview before applying.
- C12 Quick controls: one-click actions on rows.
- C13 Permissions: grants and revocation, enforced in the service layer.
- Help documents shipped with the console, rendered illustrations, and this guide.

### Not built

- Clusters that span data centers (a model cluster lives inside one site's reach).
- Autoscaling of nodes or model installs.
- A light theme.
- Offline export of the docs.
- Role bindings beyond the shipped model (member grants on sandboxes, tree roles read-only).
- Agent situation live routes (`/api/v1/live/agents*`): the page and its path stay so an
  old link still opens it, the menu entry is hidden until the routes exist.
