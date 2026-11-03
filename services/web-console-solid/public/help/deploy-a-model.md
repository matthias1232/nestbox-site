---
title: Deploy one model
slug: deploy-a-model
tags: models, deploy, fit, provider, grants
anchors: [the-catalogue, check-the-fit, deploy-the-model, the-provider-is-created-for-you, give-people-access]
---

# Deploy one model

A model deployed on a node runs on your own hardware and is offered to sandboxes like any other provider. This page shows the one-node path. For many nodes at once see [Deploy one model on twenty nodes](/docs/deploy-on-many-nodes).

## The catalogue

Open **Models** in the sidebar menu. Each card shows the model's name, family, size and licence, a fit badge for the node chosen in the header ("Fit against"), and on how many nodes it is installed. Use the chips and the search box to filter by fit, family or runtime. Switch between cards and table with the view toggle.

## Check the fit

The fit badge is computed by the server from the node's free memory and GPU memory (VRAM):

- *fits*: free VRAM is at least the recommended amount.
- *tight*: it meets the minimum but not the recommendation. It will run, with less room for long prompts.
- *no GPU* or *too small*: the node cannot run it. Pick another node.

Choose the node in the header first; the badges on every card then refer to that node.

## Deploy the model

1. Press **Deploy model** on the card, or open the model and choose it there.
2. The wizard asks for the model, then the node. Pick one node.
3. Optional settings: *force* (install even when the fit is tight) and a reserve of memory for the model.
4. On the review step, read the exact job that will start. Nothing has been written yet.
5. Finish. The install runs on the node: the image and weights are fetched, the server starts and Nestbox probes it. Progress is in the dock's *Tasks* tab and on the model page, under *Installs*.

## The provider is created for you

When an install succeeds, Nestbox creates a provider profile of the kind *local, OpenAI compatible* that points at the model on that node. You do not enter an address or a key. The model's endpoint is published on the overlay network, so a sandbox on any node can reach it, and the accounting proxy adds the credentials; a sandbox never holds a key (ADR 0057).

Uninstalling the model removes the provider. A provider used by a sandbox cannot be removed until you unlink it.

## Give people access

A model is used only by those who hold a grant. Grants have a scope:

- *Data center*: every sandbox in it. Administrators only.
- *Person*: every sandbox that person owns. Administrators only.
- *Sandbox*: one sandbox. Anyone with write access to it.

For the same model name, a sandbox grant beats a person grant, which beats a data center grant. Open the model, choose the *Grants* tab, and add a grant. Running agents are refreshed when grants change; you do not have to restart the sandbox.

## Related

- [A round-robin model cluster](/docs/model-cluster-round-robin)
- [Your sandbox](/docs/workbox) for adding a granted model to your own box
