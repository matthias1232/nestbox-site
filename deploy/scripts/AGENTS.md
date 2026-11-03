# deploy/scripts — the operator's own tools

## Purpose

Scripts an operator runs on the appliance or on a Proxmox host: bootstrap, deploy, certificates,
node images, backups, and the acceptance runs that prove a feature on the live box instead of in a
unit test. Nothing here is imported by a service; each script is started by hand or by a runbook.

## Conventions

- Shell: `#!/bin/sh` + `set -eu` (or `set -euo pipefail` where bash is required), variables quoted.
- Python: standard library only, `argparse` for inputs, no secrets in output, and a non-zero exit
  code when an assertion failed.
- An acceptance script builds its own artefacts under a unique prefix and removes them again, so
  it can run on the shared dev appliance while somebody else works on it.
- An acceptance script that starts a job tags it `params: {"source": "acceptance"}`. The API copies
  the tag into the job payload and the console's Tasks tab hides such a job, so a test run never
  fills the operator's own list. A token is never printed: where a check needs one, it is read by
  the process that already holds it and only the resulting status reaches the output.

## Files

| File | What it does |
|---|---|
| `acceptance-control-center.py` | Acceptance for the Control Center routes (tree, search, bulk, clusters, node summary, alerts, mounts, quick, preview, permissions), the accounting proxy's own model route, a bulk action that really succeeds, an allow with no end that answers an alert, the help files and the console bundle. `--environment <uuid>` sends `x-nestbox-environment` and asserts `/auth/me.environment_id` first, then stops instead of grading a data center nobody asked for. `--docker` says how to reach the Docker CLI for the proxy probe (`sudo -n docker`, or an `ssh … sudo -n docker` prefix). Prints `ok`/`WARN`/`FAIL` per check. |
| `acceptance-control-center-browser.md` | Browser checklist the coordinator walks with a signed-in user after the script passes; items 17–19 cover the dock's Tasks tab and the unlimited allow. |
| `acceptance-image-builder.py` | Acceptance for the image-builder catalogues, repositories, MCP assignments and templates. |
| `acceptance-workbox-control.py` | Acceptance for the workbox controls: agent sign-in modes, live egress mode, evaluation window, presence, model grants, tunnels, move preflight, control overview. Prints one German `CLEARAND` line per assertion. |
| `backup-control-plane.sh` | Back up PostgreSQL and, separately, the key that opens every stored secret. |
| `bootstrap-appliance.sh` | Run once as root on a fresh Ubuntu 24.04 appliance. |
| `deploy-appliance.sh` | Apply the pushed branch to the dev appliance. Refuses to start below 15 GB free on the box and prunes builder cache older than a day afterwards. |
| `make-egress-ca.sh` | Create the CA the egress proxy signs sandbox certificates with. |
| `make-node-seed.sh` | Write a NoCloud seed ISO that makes a fresh cloud image join as a node. |
| `make-proxmox-template.sh` | Prepare the Proxmox template rented nodes are cloned from. |
| `publish-node-images.sh` | Put the images a joining node needs into the registry on this host. |
| `set-inference-key.sh` | Place the inference credential where the seeded provider profile expects it. |
| `sync-egress-ca.sh` | Copy this node's egress CA into every agent image context. |
| `verify-model-serving.sh` | Acceptance: an install becomes an answering model server. |
| `verify-persistence.sh` | Acceptance: recreate the sandbox and the file is still in the home. |
