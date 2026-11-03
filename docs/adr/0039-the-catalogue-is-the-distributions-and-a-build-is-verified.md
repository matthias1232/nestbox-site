# ADR 0039 — The package catalogue is the distribution's, and a build is verified

Status: Accepted
Date: 2026-10-29

## Context

The builder could name tools from `tool_catalog`, a seed migration, and a recipe
could add packages only by their exact name. Two consequences showed up in use:

1. An operator who wanted `nginx`, `ffmpeg` or a package from their own registry
   had to know the name by heart, and an unknown name surfaced as a failed build
   minutes later.
2. A build was "successful" when the Dockerfile ran. A package the index offered
   but the image skipped — a typo in a repository, a missing `apt-get update`, a
   tool whose name differs on the baseline — left an image that looked fine and
   an agent that could not find its tool.

There was also no way to reuse a selection: every recipe repeated the same tools,
repositories and MCP servers.

## Decision

**Ask the distribution, and verify the result.**

1. `GET /api/v1/packages?baseline=&q=` runs one short-lived container of the
   baseline's own base image and asks its native package manager
   (`apt-cache search`, `dnf list --available`, `apk search`). The runner starts
   it under the same image allowlist, capability drop, memory and PID limits as a
   build, and removes it on every path. The query is validated as a package-name
   fragment and passed as an environment variable, never spliced into a shell.
   A ten-minute read-through cache in the runner is a courtesy to the index, not
   control state; `available: false` says the index could not be reached instead
   of pretending nothing matched.
2. A recipe carries `extra_packages`, `repositories` and `mcp_server_keys`
   (migration 0046). A repository is a public signing key plus an HTTPS URL; a
   private credential is a `db://` vault locator (ADR 0005). The generated
   Dockerfile writes the keyring and the repository file before the packages are
   installed, so a tool from a private registry is available without a manual
   step in the sandbox.
3. After a successful `docker build`, the runner reads the image's own package
   database (`dpkg-query`, `rpm -qa`, `apk info`) and compares it with the
   packages the recipe asked for. A missing package fails the build with the
   list; the image is not presented as usable. The check clears the image's
   entrypoint and runs without a network: a recipe image's entrypoint starts the
   supervisor, which would ignore the probe and run until the timeout, and a
   package database is local, so the check must not depend on a mirror.
4. MCP assignments are validated when the recipe is saved and attached when a
   sandbox is created from it, so an agent starts with the servers it needs.
5. `image_templates` (migration 0047) saves a selection — tools, packages,
   repositories, MCP servers — per environment, with `is_shared` and one
   `is_default` per environment. Applying a template copies the fields, moves the
   revision and clears the built image, because the previous image does not
   contain the new selection.

Alpine and Fedora became baselines (migration 0048) for the same reason: a
catalogue that offers packages the builder cannot install would be a promise it
does not keep.

## Consequences

- The catalogue needs network access from the runner, exactly as a build already
  does. It is a read of a public index in a container without capabilities, not a
  new egress path for sandboxes.
- A first search on a fresh baseline can take as long as `apt-get update`; the
  cache makes the second one immediate, and the UI says it is asking the index.
- Alpine's `apk search -v` prints `name-version - description` and `apk info -v`
  prints `name-version`; both parsers split at the first `-<digit>`, because the
  name itself contains hyphens (`py3-pip`).
- `apt-get update` inside a container with every capability dropped needs
  `APT::Sandbox::User=root`, because the default chowns its lists to `_apt` and
  `CAP_CHOWN` is gone. The capability stays dropped.
- A minimal base image has no trust store, so an HTTPS repository fails with
  "No system certificates available" before it is used. A recipe with a
  repository installs `ca-certificates` first.
- A signing key spans many lines, and a Dockerfile instruction ends at the
  newline; pasting one verbatim turns the rest into "unknown instruction". The
  generator carries the keyring and the source file as base64 on one `RUN` line
  and decodes it in the image, which every supported base image can do.
- "Built" now means "built and verified". A package that is installed under a
  different name than requested (for example a virtual package) is reported as
  missing; the fix is the real package name, which the catalogue now shows.
- A template is not a snapshot: editing it does not change recipes that already
  applied it. That is deliberate — an image must not change because a colleague
  edited a saved selection.

## Alternatives rejected

- **Scrape a distribution's web package search.** A different catalogue from the
  one the build uses, and a page layout is not a contract.
- **Seed a large package list into a table.** Stale by the next point release,
  and it silently omits everything nobody seeded.
- **Trust the Dockerfile.** It answers "did the commands run", not "does the image
  contain the tool", which is the question an operator asks.

## What would invalidate this

A distribution whose package manager has no non-interactive search, or a build
path that cannot run a container of the base image on the node. Both are
statements about the runtime, not about the feature; the catalogue would then
need a per-baseline index mirror with its own freshness rule.
