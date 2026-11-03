# ADR 0057 — The sandbox's models are granted by scope and routed per sandbox

Status: Accepted
Date: 2026-11-01

## Context

Models reached agents through one OmniRoute profile and hardcoded environment
variables: every sandbox in an environment saw the same list, nobody could give one
user a beta model, and a local model installed on a node (ADR 0018) was unreachable
from any sandbox on another node. Nothing refreshed a running agent when the model set
changed, so the answer was "restart the sandbox". Usage accounting had just learned to
read the answer (ADR 0051), but it still could not tell the proxy which upstream a
given request belonged to.

Two rules from earlier decisions shape the answer: a credential is resolved at use and
never stored in a sandbox (ADR 0016), and a node is reachable through the overlay
(ADR 0047).

## Decision

* **Grants are scoped.** `model_grants` rows have scope `environment`, `user` or
  `sandbox`. The effective list for a sandbox is the environment grants ∪ the grants of
  the sandbox's owner ∪ the sandbox's own grants; for the same `model_name` a sandbox
  row beats a user row beats an environment row. The default model is the first
  `is_default` in that order, else the first by name. Environment and user scope are
  admin-only; sandbox scope needs `SandboxWrite` on that sandbox.
* **The accounting proxy is the model router.** It already receives model traffic by
  path (ADR 0042). `POST /s/<sandbox>/r/<rest>` selects the upstream by the request
  body's `model`, answers `/s/<sandbox>/r/v1/models` in the OpenAI list format, and sets
  the `authorization` header from the provider profile. **The key never leaves the
  control plane** — the sandbox holds a router URL, not a credential.
* **Agents are pointed at the router by three variables**: `NESTBOX_MODEL_ROUTER=1`,
  `NESTBOX_MODEL_LIST=<names>`, `NESTBOX_MODEL_DEFAULT=<name>`. With the router on, the
  base URLs become `<accounting proxy>/s/<sandbox>/r` (the OpenAI base adds `/v1`), so
  every dialect an agent speaks arrives at one place.
* **A grant change reaches running agents.** Rewriting the agent configuration is live
  and all of them are refreshed, idle sessions restart when needed, and `ModelsChanged`
  goes out on the live stream so the console and the agent panels agree.
* **A local model is installed per node and published on the overlay.** Serving on a
  joined node publishes the port on the overlay address and reports it as `published`;
  a sandbox on another node reaches it through the same router entry. See ADR 0058 for
  the overlay and the egress lock a node runs under.
* **"Ready" means warmed.** A sandbox is not reported ready until its models are
  loaded and one test request has been answered; `keep_warm` pings the models marked for
  it, so the first real request does not pay the load.

## Alternatives rejected

* **Writing provider keys into sandboxes.** Against ADR 0016, and it puts a credential
  in a place a sandbox can read. The router holds it instead.
* **One proxy URL per model.** Every model change rewrites every agent configuration
  and every base URL; with one router path the configuration changes only when the
  model list does.
* **Per-node model grants only.** It makes a model a property of the machine and turns
  a sandbox move into a model migration.
* **Letting the agent choose the upstream itself.** Then the choice is neither auditable
  nor grantable, and the router cannot report what was used.

## Consequences

A model name is the unit the user sees; the upstream base URL and model id stay behind
the router. Traffic that skips the router (an agent in `subscription` mode talking to
its provider directly) is not routed and not granted — it is measured only where
ADR 0051 can see it. A model that is not in the effective list is not reachable through
the router, so a grant is the whole permission.

## Invalidation conditions

When OpenShell providers inject credentials per request, the router's authorization
step goes away and the router becomes a pure selector; the grants and the effective list
stay as they are.
