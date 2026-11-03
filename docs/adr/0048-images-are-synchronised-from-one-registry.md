# ADR 0048 — Images are synchronised from one registry, assigned by scope

Status: Accepted
Date: 2026-10-31

## Context

Every node must run identical image bytes, and a fresh cloud node must be ready quickly without uploading gigabytes from a home connection.

## Decision

1. **One OCI registry per installation**, reachable on the overlay. The image builder and recipes push there; digests stay pinned in the database.
2. **`image_assignments(digest, scope_type, scope_id)`** says which environment, site, node or label should have an image.
3. **The node agent reconciles**: pulls what is missing, reports what it holds, removes unused images after a grace period.
4. **Base layers come from the public registry** (GHCR or upstream), only our own layers come through the overlay.
5. The console shows an images-by-nodes matrix: current, pulling, missing.

## Alternatives rejected

* **Copying images with `docker save` over SSH.** No dedup, no resume.
* **Every node pulls straight from the internet.** Private builds would have to be public.

## Invalidation conditions

Revisited when one registry becomes a single point of failure for a customer.
