#!/bin/sh
# Nestbox node preparation, the first half of a join. Run as root on the machine that should
# become a node:
#   sh prepare.sh            (join.sh runs this part itself)
#
# It installs what a node needs (WireGuard tools, python3, curl, openssl, Docker) and makes this
# machine's WireGuard key if there is none yet, then prints exactly one machine-readable line,
# `NESTBOX_PUBKEY=<key>`, so the caller never has to parse prose. Running it twice is safe: an
# existing key is reused, so a half-installed machine can be finished with `apply.sh` alone.
set -eu

DIR=/opt/nestbox-node

step() { printf '\033[1m==> %s\033[0m\n' "$*"; }

[ "$(id -u)" = 0 ] || { echo "Run this as root (sudo)." >&2; exit 1; }

step "Installing the packages a node needs"
export DEBIAN_FRONTEND=noninteractive
# iproute2 and iptables are what the egress lock and the tunnel readiness check use, and ping is
# not part of a minimal Debian: wireguard-tools alone does not bring them in on every image.
if command -v apt-get >/dev/null 2>&1; then
  apt-get update -qq
  apt-get install -y -qq wireguard-tools iproute2 iptables iputils-ping python3 curl ca-certificates openssl >/dev/null
else
  for tool in wg python3 curl openssl ip ping; do
    command -v "$tool" >/dev/null 2>&1 || { echo "Install '$tool' first; this installer knows apt only." >&2; exit 1; }
  done
fi

step "Installing Docker"
if ! command -v docker >/dev/null 2>&1; then
  curl -fsSL https://get.docker.com | sh >/dev/null
fi
# The kill switch of the egress lock lives in Docker's own DOCKER-USER chain, so the daemon has to
# be running before apply.sh gets to that point.
systemctl enable --now docker >/dev/null 2>&1 || true

step "Making this node's WireGuard key"
# The private half never leaves this machine; only the public half goes to the control plane.
# The umask keeps it 0600 whatever the file mode of the day is.
umask 077
mkdir -p "$DIR/secrets" /etc/wireguard
[ -s /etc/wireguard/nb0.key ] || wg genkey > /etc/wireguard/nb0.key

echo "NESTBOX_PUBKEY=$(wg pubkey < /etc/wireguard/nb0.key)"
