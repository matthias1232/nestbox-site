---
title: Deploy one model on twenty nodes
slug: deploy-on-many-nodes
tags: models, nodes, bulk, cluster, deploy
anchors: [pick-the-model, choose-nodes, set-the-options, review-and-deploy, watch-progress, make-a-cluster]
---

# Deploy one model on twenty nodes

Example: you have just added twenty GPU nodes to a site and you want DeepSeek on every one that can run it. One wizard does it as one job.

## Pick the model

1. Open **Models** in the sidebar menu.
2. Search for "deepseek" and choose the model, for example *DeepSeek R1 7B*. Press **Deploy model**.
3. The wizard opens on the *Model* step with your choice selected. Press Next.

You can also open a model's page and press the deploy action there; the model is then preselected.

## Choose nodes

The *Nodes* step lists your nodes grouped by data center and then by site, each with a fit badge and a checkbox.

1. Press **All that fit**. Every online node that meets the recommended memory is ticked.
2. To limit the choice, use **Whole site** or **Whole data center**, or untick single nodes.
3. Read the count line, for example "12 selected · 2 tight · 3 skipped". *Skipped* nodes are offline, have no GPU, or already have the model; they are listed with the reason and are not counted in the job.

Tight nodes are ticked only if you tick them yourself.

## Set the options

- **Force**: install on tight nodes too.
- **Make these a cluster**: after the installs, group them behind one address. Give the cluster a name and a strategy. See [A round-robin model cluster](/docs/model-cluster-round-robin).
- **Grant to this data center**: lets every sandbox in the data center use the model.

## Review and deploy

The *Review* step lists every node and the exact job that will start, plus the cluster that will exist. Nothing is written until you finish. The button is labelled with the number of targets, for example **Deploy on 20 nodes**. Press it.

## Watch progress

The wizard turns into a progress view: one row per node with its own bar (installing, installed or failed), a last log line, and an overall bar. The same job is in the dock's *Tasks* tab, so you can close the wizard and carry on.

- A failed node does not stop the others. Open its row to read the log, fix the cause (usually disk space or an unreachable node) and run the wizard again; nodes that already have the model are skipped.
- A provider profile appears for each node as its install succeeds. Find them under *Installs* on the model's page.

## Make a cluster

If you ticked *Make these a cluster*, the job creates the cluster when at least one install has succeeded and an **Open cluster** button appears. If you did not, open the model page and press **Make these a cluster** (it appears when the model has two or more installs). Choose the installs, a name and a strategy.

Sandboxes then use one address instead of twenty. Details, strategies and how to point an agent at it are in [A round-robin model cluster](/docs/model-cluster-round-robin).

## Related

- [Deploy one model](/docs/deploy-a-model)
- [Nodes](/docs/nodes)
