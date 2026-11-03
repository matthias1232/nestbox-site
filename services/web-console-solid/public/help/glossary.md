---
title: Glossary
slug: glossary
tags: glossary, terms, definitions
anchors: [sandbox, node, site, data-center, organisation, policy, window, grant, model-cluster, egress-mode, openshell, gateway, supervisor]
---

# Glossary

Short definitions of the words the console uses.

## Sandbox

An isolated workload (a container) in which an agent or tool runs. It has its own files, a policy and one node it runs on. Its name is unique on the whole installation.

## Node

A machine that runs sandboxes. It joins the control plane through an encrypted tunnel. See [Nodes](/docs/nodes).

## Site

What vCenter calls a cluster: the nodes of one place or provider inside a data center. A site is *home*, *cloud* or *edge*.

## Data center

Where one team runs its sandboxes. It is the isolation boundary: data of one data center is invisible to another.

## Organisation

A customer or provider. Organisations form a tree; each owns one or more data centers. See [Customers, data centers, sites](/docs/tenant-tree).

## Policy

The rules of what a sandbox may reach and touch, in four sections (network, filesystem, landlock, process). See [Policies and the wizard](/docs/policies).

## Window

A time-limited (or unlimited, when you choose so) exception on one channel that allows traffic that rules would block. Used to try something. See [Egress modes and observation](/docs/egress-modes).

## Grant

A right to use something. A model grant gives a data center, a person or a sandbox access to a model. A permission grants a role on a sandbox.

## Model cluster

Several installs of one model on different nodes behind one address, with requests spread round robin (or by least load, or failover). See [A round-robin model cluster](/docs/model-cluster-round-robin).

## Egress mode

How outgoing traffic of a sandbox is seen and decided: forward proxy, transparent, hybrid, or off (allow all).

## OpenShell

The NVIDIA project that Nestbox wraps. It runs the sandbox and enforces the policy document. Nestbox adds the database, the console and the telemetry around it.

## Gateway

The OpenShell service that manages sandboxes and decides their connections at the network layer.

## Supervisor

The OpenShell process inside a sandbox that applies the policy and connects the sandbox to the gateway. If it stops, the sandbox loses its tools.

## Related

- [Getting started](/docs/getting-started)
