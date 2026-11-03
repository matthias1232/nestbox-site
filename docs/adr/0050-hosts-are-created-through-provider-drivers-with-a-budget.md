# ADR 0050 — Hosts are created through provider drivers, with a lifetime and a budget

Status: Accepted
Date: 2026-10-31

## Context

The goal is to start a machine at any cheap host, use it, and delete it, without surprise cost.

## Decision

1. **A `ProviderDriver` trait**: `list_types`, `price`, `create(cloud_init)`, `status`, `destroy`. Credentials are vault locators.
2. **Order**: Proxmox (own infrastructure, first test), NVIDIA Brev, Hetzner, generic SSH; others later. A host without an API uses the cloud-init text or the ISO.
3. **Burst** is one flow: create, join, sync images, move the sandbox, work, move back, dirty check, destroy.
4. **Cost guards**: a lifetime on every created node, stop when idle, a budget per data center, and the hourly price shown before creating.
5. **Distribution**: signed multi-arch images on GHCR, a compose bundle and an installer per release, documentation on GitHub Pages. The hosted service runs the same code on a real server, because Pages is static.

## Alternatives rejected

* **One cloud only.** Defeats "any cheap host".
* **Terraform inside the product.** A second state store beside PostgreSQL.

## Invalidation conditions

Revisited when a provider has no usable API or when billing becomes a product feature.

## Amendment 2026-11-01 - what was built

* Drivers: **Hetzner Cloud** (join command in `user_data`; request shapes and price parsing are unit
  tested, not run against a real account) and **Proxmox VE** (clone a template, address from a pool,
  run the join command through the guest agent; exercised end to end against a real Proxmox host).
* **DigitalOcean and Vultr** drivers follow the providers' published v2 APIs (droplet / instance with the
  join command as user data, hourly prices from the size / plan lists); request shapes and answer parsing are
  unit tested, they have not been run against live accounts.
* **Not built:** NVIDIA Brev (no API to verify against here; a Brev instance joins with the one-line
  command or the cloud-init document, like any other host), AWS, GCP, and a generic SSH driver (the
  container has no SSH client; the one-line command covers it).
* The reaper deletes a machine when its lifetime ends or it has been empty for its idle limit, never
  while data is on it (the deadline moves by 30 minutes and the console says why), and repeats the join
  step for a machine that never became a node.
* The hosted service, GitHub Pages: Pages serves the static site that already lives in this repository;
  the hosted control plane is an ordinary installation with `NESTBOX_MODE=saas`.
