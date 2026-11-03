---
title: A round-robin model cluster
slug: model-cluster-round-robin
tags: models, cluster, round robin, failover, provider
anchors: [what-a-cluster-is, example-four-deepseek-nodes, strategies, health, use-it-in-a-sandbox]
---

# A round-robin model cluster

A model cluster puts several installs of the same model behind one address. Requests are spread over the members, and a broken member is skipped.

![Model cluster](/illustrations/docs-model-cluster.webp)

## What a cluster is

A cluster belongs to one model and one data center. Its members are installs of that model on nodes of the same data center. Creating a cluster also creates one provider profile with the cluster's name; its address is the accounting proxy's cluster path, and it holds no secret. That provider is what sandboxes use.

## Example: four DeepSeek nodes

You deployed DeepSeek R1 7B on four nodes (see [Deploy one model on twenty nodes](/docs/deploy-on-many-nodes)) and want one endpoint.

1. Open the model's page and press **Make these a cluster**, or tick the option in the deploy wizard.
2. Name it, for example `deepseek-x4`, and choose *Round robin*.
3. Tick the four installs and confirm.
4. Open the cluster (**Open cluster**). You see each member with its node, state, provider status and weight, and the cluster's endpoint.

Add or remove members later on the cluster page. A member is removed from rotation at once.

## Strategies

| Strategy | Behaviour |
|---|---|
| Round robin | Takes members in turn, weighted by each member's weight (1 to 100). Good default for equal nodes. |
| Least loaded | Sends the request to the member with the fewest requests in flight right now. |
| Failover | Always the first healthy member in order; the rest wait as spares. |

You can change the name and strategy of a cluster, not its model.

## Health

A member counts as healthy only when its install is *installed* and its provider status is *reachable*. If a connection to a member fails, the next member is tried once. Each answer carries the header `x-nestbox-cluster-member` naming the install that served it, and usage is recorded against that install. The cluster page shows "healthy members: 3 of 4".

## Use it in a sandbox

1. Open the cluster page and read "How a sandbox uses it", or open the model page's *Grants* tab.
2. Grant the cluster's provider to the data center, a person or one sandbox, as for any model (see [Deploy one model](/docs/deploy-a-model)).
3. Running agents pick up the grant within moments.
4. The model name an agent asks for is the model key, for example `deepseek-r1-7b`. It is the same name whichever member answers. In a sandbox, `NESTBOX_MODEL_LIST` shows the names granted and `NESTBOX_MODEL_DEFAULT` the default.

If you remove the cluster, its provider is removed too, unless a sandbox still uses it; then the removal is refused until you unlink it.

## Related

- [Nodes](/docs/nodes)
- [Glossary](/docs/glossary)
