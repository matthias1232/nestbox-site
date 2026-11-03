# Deploy

## What runs here

| Service | Role |
|---|---|
| `postgres` | The single control store (ADR 0001) |
| `migrate` | Applies database migrations, then exits |
| `control-api` | The Rust API |
| `web-console` | The SolidJS console, served by nginx |
| `sandbox-runner` | The only component with Docker socket access |
| `gateway` | The OpenShell-facing gateway image |
| `egress-proxy` | The only way out of the sandbox network; asks the API per request |

## Secrets

Secrets are **files**, not environment variables, because environment variables
appear in `docker inspect` and in crash dumps.

```bash
mkdir -p secrets
chmod 0700 secrets
printf '%s' "$(openssl rand -base64 32)" > secrets/postgres_password
PGPW="$(cat secrets/postgres_password)"
# The API runs as the least-privilege role, so row-level security applies to it.
printf '%s' "postgres://nestbox_app:${PGPW}@postgres:5432/nestbox" > secrets/database_url
# Migrations need the schema owner. This file is mounted only into `migrate`.
printf '%s' "postgres://nestbox:${PGPW}@postgres:5432/nestbox" > secrets/database_url_admin

# The node token authenticates the control API to the runner's terminal
# endpoint. It is the only file in `secrets/node/` on purpose: the runner mounts
# that directory and must not be able to read the database URLs beside it.
# `init-data` generates it on first start, so this is only needed if you want it
# before the first `up`.
# Hex, not base64: the token is sent as a `Sec-WebSocket-Protocol` entry, and
# base64's `+`, `/` and `=` are not valid there.
install -d -m 0755 secrets/node
head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' > secrets/node/token

chmod 0600 secrets/* secrets/node/*
```

The browser terminal is a WebSocket from the console to a PTY in the sandbox. It
is refused unless both services hold the same node token, which is the point:
the runner also sits on the sandbox network, so an unauthenticated endpoint there
would be a shell in every neighbour. The API reads the file when a terminal
opens, so a rotated token applies to the next session; terminals already open
keep running until they end.

The API reads `DATABASE_URL_FILE` rather than `DATABASE_URL`, so the connection
string never becomes a process argument.

The two URLs are the reason row-level security is real rather than decorative:
`nestbox_app` does not own the tables, so PostgreSQL enforces the tenant policy
on it. `nestbox` is the owner and is used only by migrations and by the
cross-tenant maintenance jobs.

Provider credentials are **not** placed in the compose file. They live in
`secrets/` and the database holds only a `locator` pointing at them (ADR 0005).

### The inference credential

Migration `0011` seeds one provider profile (`omniroute`) and one locator
(`file:///secrets/omniroute_api_key`). The file itself is the operator's to
place, because a key in a migration would be a key in the repository:

```bash
deploy/scripts/set-inference-key.sh          # prompts, writes 0600, never echoes
# or non-interactively:
OMNIROUTE_API_KEY=... deploy/scripts/set-inference-key.sh
```

The API resolves the locator when it creates an agent sandbox and passes the
value to the container as `OPENAI_API_KEY` together with the profile's endpoint
and model. It is never written to the database, never returned by an endpoint,
and never logged — `security::secrets` refuses a locator outside `secrets/` and
refuses a file that group or world can read.

Rotating the key is: write the new value to the same file, recreate the sandbox.
Nothing else changes, and no rebuild is needed.


## The egress proxy and its CA

`sandbox-runner`'s sandboxes sit on `runtime`, which is `internal: true`: they
have no default route. Their only exit is `egress-proxy`, an Envoy forward proxy
that asks `control-api` for a decision on every request. No rule, no window, no
exit — that is the default-deny promise, and a control API that cannot answer is
a deny, not a pass.

The proxy itself owns no rules and is not published on the host. It is the one
container with `NET_ADMIN` and `NET_RAW`, and only for the redirect rules that
catch a tool which ignores `HTTP_PROXY`.

To let the proxy inspect HTTPS instead of only tunnelling it, every node needs
its own CA:

```bash
deploy/scripts/make-egress-ca.sh      # idempotent; --force replaces
```

It writes `secrets/egress/ca.key` (0600, node only) and `secrets/egress/ca.crt`.
The certificate goes into every sandbox image as a trusted root; the key never
leaves the node and never enters PostgreSQL (ADR 0005). Without it, an HTTPS
request is logged as a CONNECT tunnel with host, port, SNI, and byte counts, and
the console says exactly that rather than showing an empty path.

## Start

```bash
docker compose up -d --build
docker compose logs -f control-api
```

The console is on `:8080`. The API is not published on the host: the console
proxies it, so there is one entry point and one place to add TLS later.

## First start

The migration seeds:

- one organization and one environment named `default`
- the model catalog
- the operational schedules (sync, reconciliation, retention, expiry checks)
- one admin user, whose initial password is printed once to the migration log

Change that password immediately. It is not stored anywhere retrievable.

## Verify

```bash
curl -fsS http://localhost:8080/api/v1/healthz
curl -fsS http://localhost:8080/api/v1/readyz
```

`readyz` checks the database connection, not just that the process is alive.

## Rebuild from scratch

The development environment is disposable (ADR 0001). To start over:

```bash
docker compose down -v      # -v drops the database volume
docker compose up -d --build
```

## Backup

```bash
docker compose exec -T postgres \
  pg_dump -U nestbox -Fc nestbox > nestbox-$(date +%F).dump
```

Restore with `pg_restore`. Secrets are backed up separately: they are files, and
a database backup deliberately does not contain them.

## Upgrading

Migrations run before the API starts, so an upgrade is:

```bash
git pull
docker compose up -d --build
```

`migrate` applies forward-only migrations. A failed migration stops the rollout
rather than leaving a half-upgraded schema.

## Tests

`docker-compose.test.yml` starts a throwaway PostgreSQL for the integration
tests:

```bash
docker compose -f docker-compose.test.yml up -d
export DATABASE_URL=postgres://nestbox:test@localhost:55432/nestbox_test
cargo test
```

## Multi-tenant and multi-node (ADR 0044-0050)

| Piece | Where |
|---|---|
| One-line installer | `install.sh` (see `docs/guides/self-hosting.md`) |
| WireGuard hub for joined nodes | `overlay-hub/`, compose service `overlay-hub` (host network, `NET_ADMIN`) |
| Image registry for nodes | compose service `registry`, listens on the loopback, forwarded to the overlay by the hub |
| Snapshot server (restic, append-only) | compose service `restic-server` |
| Publish images for nodes | `scripts/publish-node-images.sh` |
| Proxmox template for rented machines | `scripts/make-proxmox-template.sh` (run on the Proxmox host) |
| Seed ISO that makes a VM join | `scripts/make-node-seed.sh` |
| Control-plane backup | `scripts/backup-control-plane.sh` |
| Signed release images | `docker-compose.images.yml`, built by `.github/workflows/release.yml` |

Settings in `deploy/.env`: `NESTBOX_MODE`, `NESTBOX_PUBLIC_URL`, `NESTBOX_OVERLAY_ENDPOINT`
(`host:51821`, UDP, reachable from the nodes), `NESTBOX_OIDC_PROVIDERS` and the per-provider client
settings. Joining nodes is off while `NESTBOX_OVERLAY_ENDPOINT` is empty.

The hub forwards the shared services (MCP hub, media, accounting, egress proxy) to every node and
routes a strict-locked node's traffic out (ADR 0057).
