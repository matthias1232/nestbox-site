# Architecture Decision Records

Each file here records one decision: the context, the choice, the alternatives
that were rejected and why, the consequences, and what would invalidate it.

ADRs are immutable once accepted. To change a decision, write a new ADR that
supersedes the old one and note the supersession in both files.

| Number | Title | Status |
|---|---|---|
| [0001](0001-postgresql-as-only-control-store.md) | PostgreSQL as the only control store | Accepted |
| [0002](0002-layered-architecture-and-enforced-boundaries.md) | Layered architecture with enforced boundaries | Accepted |
| [0003](0003-documentation-as-a-build-artifact.md) | Documentation as a build artifact | Accepted |
| [0004](0004-session-auth-rbac-and-tenant-scope.md) | Session auth, RBAC, and tenant scope | Accepted |
| [0005](0005-secrets-as-locators-only.md) | Secrets as locators only | Accepted |
| [0006](0006-cursor-paging-and-facets.md) | Cursor paging and server-side facets | Accepted |
| [0007](0007-policy-scope-mirrors-openshell.md) | Policy scope mirrors OpenShell | Accepted |
| [0008](0008-hot-reloadable-network-section.md) | Only the network section is hot-reloadable | Accepted |
| [0009](0009-event-bus-behind-a-trait.md) | The event bus sits behind a trait | Accepted |
| [0010](0010-telemetry-partitioned-by-month.md) | Telemetry is partitioned by month | Accepted |
| [0011](0011-rule-from-event-is-a-two-step-workflow.md) | Rule from event is a two-step workflow | Accepted |
| [0012](0012-gateway-settings-are-not-policy.md) | Gateway settings are not policy | Accepted |
| [0013](0013-model-catalog-requirements-are-data.md) | Model requirements are data | Accepted |
| [0014](0014-openshell-cli-feature-parity.md) | OpenShell CLI feature parity | Accepted |
| [0015](0015-published-api-contract-is-binding.md) | The published API contract is binding | Accepted |
| [0016](0016-provider-credentials-resolved-at-use.md) | Provider credentials are resolved at use, never stored or returned | Accepted |
| [0017](0017-server-computed-bars-and-honest-unified-memory.md) | Resource bars are server-computed, and unified memory is recognised, never inferred | Accepted |
| [0018](0018-a-model-is-an-install-on-a-node.md) | A model is an install on a node, and the fit is measured against what is free | Accepted |
| [0019](0019-catalog-sync-derives-by-rule.md) | The catalog is derived from the repositories by rule, and a curated row wins | Accepted |
| [0020](0020-packs-are-data.md) | Baselines, agent packs, and tools are data | Accepted |
| [0021](0021-persistence-is-bind-mounts.md) | Persistent data lives in bind mounts, one read-write mount per root | Accepted |
| [0022](0022-egress-is-one-proxy-default-deny-fail-closed.md) | One egress proxy per node, default deny, fail closed | Accepted |
| [0023](0023-terminal-sessions-are-audited-without-content.md) | A terminal session is audited by who and when, never by what | Accepted |
| [0024](0024-ssh-key-is-a-one-time-download.md) | The sandbox SSH key is a one-time download, and the database holds a locator | Accepted |
| [0025](0025-sandbox-metrics-live-in-postgresql.md) | Sandbox metrics live in PostgreSQL, raw for a day and rolled up for a week | Accepted |
| [0026](0026-mcp-calls-are-traffic-and-stdio-is-not-egress.md) | An MCP call over HTTP is traffic; a stdio server is not egress | Accepted |
| [0027](0027-bodies-are-logged-fully-and-the-redactor-is-the-boundary.md) | Bodies are logged fully, and the redactor is the boundary | Accepted |
| [0028](0028-model-serving.md) | An install serves one artefact from one pinned image | Accepted |
| [0029](0029-a-node-is-a-row-plus-an-adapter.md) | A node is a row plus an adapter; multi-node is not a rename | Superseded by 0047 |
| [0030](0030-sandboxes-that-belong-together-are-linked-explicitly.md) | Sandboxes that belong together are linked explicitly, one level deep | Accepted |
| [0031](0031-a-sandbox-is-reached-through-its-nodes-address.md) | A sandbox is reached through its node's address, never the container's | Accepted |
| [0032](0032-the-stack-is-drawn-from-facts-and-helpers-link-themselves.md) | The stack diagram is arranged from facts, and a lone helper links itself | Accepted |
| [0033](0033-published-ports-are-held-open-by-the-runner.md) | Published ports are held open by the runner, and SSH is answered by it | Accepted |
| [0034](0034-the-proxy-identifies-by-address-and-asks-before-it-tunnels.md) | The proxy identifies a sandbox by its address, and asks before it tunnels | Accepted |
| [0035](0035-a-sandbox-is-administered-live-and-starts-connected.md) | A sandbox is administered live, and its agent starts connected | Accepted |
| [0036](0036-sandboxes-run-on-openshell.md) | Sandboxes run on the NVIDIA OpenShell gateway | Accepted |
| [0037](0037-keys-typed-into-the-console-are-stored-encrypted.md) | Keys typed into the console are stored encrypted, write-only | Accepted |
| [0038](0038-a-user-owns-their-sandboxes.md) | A user owns their sandboxes; an admin sees and sets everything shared | Accepted |
| [0039](0039-media-rendering-is-a-governed-capability.md) | Media rendering is a governed capability, not a provider key in the sandbox | Accepted |
| [0039](0039-the-catalogue-is-the-distributions-and-a-build-is-verified.md) | The package catalogue is the distribution's, and a build is verified | Accepted |
| [0040](0040-a-media-release-is-per-person.md) | A media release is per person, not only per environment | Accepted |
| [0041](0041-name-lookups-and-bounded-master-switches.md) | A name lookup is its own event, and a master switch is a bounded window | Accepted |
| [0042](0042-model-traffic-is-routed-through-the-accounting-proxy-by-path.md) | Model traffic is routed through the accounting proxy, and the sandbox rides in the path | Accepted |
| [0043](0043-a-sandbox-can-have-a-desktop-and-is-watched-live.md) | A sandbox can have a desktop, and is watched live | Accepted |
| [0044](0044-organisations-data-centers-and-sites-are-the-tenant-tree.md) | Organisations, data centers and sites are the tenant tree | Accepted |
| [0045](0045-access-is-a-role-binding-with-a-scope.md) | Access is a role binding with a scope, and a sandbox can be delegated | Accepted |
| [0046](0046-sign-in-through-an-identity-provider.md) | Sign-in through an identity provider, and who may register | Accepted |
| [0047](0047-nodes-join-through-an-agent-and-an-overlay.md) | Nodes join through an agent and a WireGuard overlay; this replaces ADR 0029 | Accepted |
| [0048](0048-images-are-synchronised-from-one-registry.md) | Images are synchronised from one registry, assigned by scope | Accepted |
| [0049](0049-sandboxes-move-as-snapshots-and-roots-have-one-owner-node.md) | Sandboxes move as snapshots, and a root lives on one node at a time | Accepted |
| [0050](0050-hosts-are-created-through-provider-drivers-with-a-budget.md) | Hosts are created through provider drivers, with a lifetime and a budget | Accepted |
| [0051](0051-model-usage-is-read-from-the-answer-and-keys-and-accounts-are-kept-apart.md) | Model usage is read from the answer, and keys and accounts are kept apart | Accepted |
| [0052](0052-gateway-sandboxes-run-through-the-gateway-and-the-proxy-chain-does-not-reach-them.md) | Gateway sandboxes are driven through the gateway, and the proxy chain does not reach them | Accepted |
| [0053](0053-the-egress-mode-is-decided-per-request-and-the-ca-is-always-mounted.md) | The egress mode is decided per request, and the CA is always mounted | Accepted |
| [0054](0054-allow-everything-is-loud-and-bounded-by-choice.md) | "Allow everything" states are loud, bounded by choice | Accepted |
| [0055](0055-an-agents-sign-in-mode-lives-in-a-launcher.md) | An agent's sign-in mode lives in a launcher, not the container | Accepted |
| [0056](0056-a-sandbox-session-is-shared-by-default.md) | A sandbox session is shared by default, and watching is visible | Accepted |
| [0057](0057-models-are-granted-by-scope-and-routed-per-sandbox.md) | The sandbox's models are granted by scope and routed per sandbox | Accepted |
| [0058](0058-one-gateway-decisions-in-the-middle-bytes-in-a-tunnel.md) | One gateway: decisions in the middle, bytes in a tunnel | Accepted |
| [0059](0059-content-inspection-on-the-decrypted-hop.md) | Content is inspected on the decrypted hop, in the broker, not in a filter | Accepted |
| [0060](0060-a-rule-may-expire.md) | A rule may expire, and the expiry lives in the database, not in the struct | Accepted |
| [0061](0061-pseudonymise-and-restore-on-the-way-back.md) | Pseudonymise on the way out, restore on the way back | Accepted |
| [0062](0062-a-finding-can-be-allowed-on-purpose.md) | A finding can be allowed on purpose, and an exception is a row, not a mode change | Accepted |
| [0063](0063-who-may-read-a-chat.md) | Who may read a chat, and what the reader leaves behind | Accepted |
| [0064](0064-the-console-is-an-inventory-tree-with-real-addresses.md) | The console is an inventory tree, and every view has a real address | Accepted |
| [0065](0065-a-model-cluster-is-installs-behind-one-provider-routed-by-the-proxy.md) | A model cluster is installs behind one provider, routed by the proxy | Accepted |
| [0066](0066-an-alert-is-a-refused-destination-awaiting-an-answer.md) | An alert is a refused destination awaiting an answer | Accepted |
| [0067](0067-help-and-docs-ship-inside-the-console.md) | Help and documentation ship inside the console | Accepted |
| [0068](0068-a-build-outlives-the-call-that-started-it.md) | A recipe build outlives the call that started it, and its Dockerfile carries the pack | Accepted |

## Writing a new ADR

Copy the shape of an existing file. Keep it short. The value is in the
*rejected alternatives* and the *invalidation conditions*, because those are
what a future reader cannot reconstruct from the code.
