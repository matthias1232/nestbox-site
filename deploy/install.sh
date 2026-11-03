#!/usr/bin/env bash
# Install Nestbox on one machine you control (ADR 0050).
#
#   curl -fsSL https://raw.githubusercontent.com/matthias1232/nestbox-site/main/deploy/install.sh | sudo bash
#   NESTBOX_VERSION=v1.0.0 NESTBOX_MODE=saas NESTBOX_PUBLIC_URL=https://nestbox.example sudo -E bash install.sh
#
# What it does: checks for Docker, fetches the release into /opt/nestbox, writes deploy/.env, starts
# the control plane (database, API, console, runner, egress proxy, node hub, image registry,
# snapshot server) and prints the address and the administrator's one-time password.
#
# Settings (environment variables, all optional):
#   NESTBOX_HOME        where it lives (default /opt/nestbox)
#   NESTBOX_VERSION     a release tag or branch to install (default: main)
#   NESTBOX_REPO        the git URL to fetch (default: the project repository)
#   NESTBOX_MODE        selfhosted (default: invitations only) or saas (sign-up by identity provider)
#   NESTBOX_PUBLIC_URL  the address people and nodes use, for example http://203.0.113.7:8080
#   NESTBOX_IMAGES      ghcr to pull the released images instead of building from source
set -euo pipefail

HOME_DIR="${NESTBOX_HOME:-/opt/nestbox}"
VERSION="${NESTBOX_VERSION:-main}"
REPO="${NESTBOX_REPO:-https://github.com/matthias1232/nestbox-site.git}"
MODE="${NESTBOX_MODE:-selfhosted}"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run this as root (sudo)."
command -v git >/dev/null 2>&1 || { say "Installing git"; apt-get update -qq && apt-get install -y -qq git ca-certificates curl >/dev/null; }
if ! command -v docker >/dev/null 2>&1; then
  say "Installing Docker"
  curl -fsSL https://get.docker.com | sh >/dev/null
fi
docker compose version >/dev/null 2>&1 || die "Docker Compose v2 is required (docker compose version)."
case "$MODE" in selfhosted|saas) ;; *) die "NESTBOX_MODE is selfhosted or saas." ;; esac

if [ -d "$HOME_DIR/.git" ]; then
  say "Updating $HOME_DIR to $VERSION"
  git -C "$HOME_DIR" fetch --quiet --tags origin
  git -C "$HOME_DIR" checkout --quiet "$VERSION"
  git -C "$HOME_DIR" merge --ff-only --quiet "origin/$VERSION" 2>/dev/null || true
else
  say "Fetching Nestbox $VERSION into $HOME_DIR"
  git clone --quiet --branch "$VERSION" --depth 1 "$REPO" "$HOME_DIR"
fi

cd "$HOME_DIR/deploy"
address="${NESTBOX_PUBLIC_URL:-}"
if [ -z "$address" ]; then
  host="$(hostname -I 2>/dev/null | awk '{print $1}')"
  address="http://${host:-localhost}:8080"
fi
public_host="$(printf '%s' "$address" | sed -E 's#^[a-z]+://##; s#[:/].*$##')"

if [ ! -f .env ]; then
  say "Writing deploy/.env"
  umask 077
  cat > .env <<ENV
# Written by install.sh. Edit and run: docker compose up -d
NESTBOX_MODE=$MODE
NESTBOX_PUBLIC_URL=$address
# Where nodes on other machines dial in (UDP). Open it in the firewall if you will add nodes.
NESTBOX_OVERLAY_ENDPOINT=$public_host:51821
# Sign-in providers (saas or selfhosted), for example: NESTBOX_OIDC_PROVIDERS=google,github
# with NESTBOX_OIDC_GOOGLE_CLIENT_ID / NESTBOX_OIDC_GOOGLE_CLIENT_SECRET etc. The redirect URI to
# register at the provider is: $address/api/v1/auth/oidc/<provider>/callback
NESTBOX_OIDC_PROVIDERS=
ENV
else
  say "deploy/.env exists; keeping it"
fi

# Secrets are files, not environment variables (ADR 0005). Made once; never overwritten.
if [ ! -f secrets/postgres_password ]; then
  say "Generating the database secrets"
  umask 077
  mkdir -p secrets/node
  password="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
  printf '%s' "$password" > secrets/postgres_password
  # The API connects as the least-privilege role, so row-level security applies to it;
  # migrations connect as the schema owner.
  printf '%s' "postgres://nestbox_app:${password}@postgres:5432/nestbox" > secrets/database_url
  printf '%s' "postgres://nestbox:${password}@postgres:5432/nestbox" > secrets/database_url_admin
  head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' > secrets/node/token
  chmod 0755 secrets/node
  chmod 0644 secrets/node/token
fi

compose=(docker compose)
up_args=(up -d)
if [ "${NESTBOX_IMAGES:-}" = ghcr ]; then
  compose=(docker compose -f docker-compose.yml -f docker-compose.images.yml)
  up_args=(up -d --no-build)
  export NESTBOX_VERSION="$VERSION"
  say "Pulling the released images"
  "${compose[@]}" pull --quiet
else
  say "Building from source (about 10 minutes the first time)"
fi

services=(migrate control-api web-console sandbox-runner egress-proxy overlay-hub registry restic-server)
"${compose[@]}" "${up_args[@]}" --build "${services[@]}" 2>/dev/null || "${compose[@]}" "${up_args[@]}" "${services[@]}"

say "Waiting for the API"
for _ in $(seq 1 60); do
  curl -fsS "http://127.0.0.1:8080/api/v1/healthz" >/dev/null 2>&1 && break
  sleep 3
done
curl -fsS "http://127.0.0.1:8080/api/v1/healthz" >/dev/null 2>&1 || die "the API did not come up; see: docker compose -f $HOME_DIR/deploy/docker-compose.yml logs control-api"

password="$(docker compose logs migrate 2>/dev/null | sed -n 's/.*password: //p' | tail -1)"
echo
say "Nestbox is running at $address"
if [ -n "$password" ]; then
  echo "    Sign in as:  admin@nestbox.local"
  echo "    Password:    $password"
  echo "    (shown once; change it under Users)"
else
  echo "    The administrator password was printed on the first start:"
  echo "    docker compose -f $HOME_DIR/deploy/docker-compose.yml logs migrate | grep password"
fi
echo
echo "    Add a node:     console -> Nodes -> Add a node (needs UDP ${public_host}:51821 reachable)"
echo "    Back up:        $HOME_DIR/deploy/scripts/backup-control-plane.sh"
echo "    Documentation:  $HOME_DIR/docs/guides/"
