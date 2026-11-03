#!/usr/bin/env bash
# Nestbox node join. Run as root on the machine that should become a node:
#   curl -fsSL "__CONTROL_URL__/api/v1/join/__TOKEN__/script" | sudo bash
#
# What it does, in order: prepares the machine (packages, Docker, this machine's WireGuard key),
# trades the one-time token for an identity, brings up the tunnel to the control plane, applies
# the egress lock, and starts the node's runner and egress proxy from images the control plane
# serves. It is safe to run again: the token is single-use, so a second run stops with a clear
# message instead of making a second node.
#
# It is three pieces in one file. The control plane pastes the prepare half in place of
# __PREPARE__ and the apply half in place of __APPLY__; the middleware below turns the join
# answer into the NB_* variables the apply half reads, so the same apply half runs a join and a
# "connect by IP" over SSH. Keep the two placeholders on their own line.
set -euo pipefail

CONTROL="__CONTROL_URL__"
TOKEN="__TOKEN__"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
need_root() { [ "$(id -u)" = 0 ] || { echo "Run this as root (sudo bash)." >&2; exit 1; }; }
need_root

__PREPARE__

PUB=$(wg pubkey < /etc/wireguard/nb0.key)

say "Joining the control plane"
REQUEST=$(python3 - "$PUB" "$(hostname)" <<'PY'
import json, sys
print(json.dumps({"public_key": sys.argv[1], "hostname": sys.argv[2]}))
PY
)
ANSWER=$(curl -fsS -X POST -H 'content-type: application/json' -d "$REQUEST" "$CONTROL/api/v1/join/$TOKEN") || {
  echo "The control plane refused the join. The link is single-use and expires; ask for a new one." >&2
  exit 1
}
field() { python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1" <<<"$ANSWER"; }

# The join answer, in the names the apply half reads. Nothing is parsed twice: a field that is
# missing here is a control plane that does not answer the shape this script was built for.
NB_NODE_NAME=$(field node_name)
NB_OVERLAY_IP=$(field overlay_ip)
NB_HUB_PUBLIC_KEY=$(field hub_public_key)
NB_HUB_ENDPOINT=$(field hub_endpoint)
NB_HUB_IP=$(field hub_ip)
NB_REGISTRY=$(field registry)
NB_CONTROL_URL=$(field control_url)
NB_NODE_TOKEN=$(field node_token)
NB_NODE_ID=$(field node_id)
NB_ENVIRONMENT_ID=$(field environment_id)
# A control plane that has never heard of the lock answers without the field; a node that is not
# told otherwise keeps the split tunnel it always had, so an old control plane still works.
NB_EGRESS_LOCK=$(python3 -c 'import json,sys
lock = json.load(sys.stdin).get("egress_lock") or "exit_local"
print(lock if lock in ("strict", "exit_local") else "exit_local")' <<<"$ANSWER")
# Empty: this node dials the hub, which is what a join from behind any NAT can do. A hub-dialled
# node is told its port by whoever registered it from the control plane side.
NB_LISTEN_PORT=""
COMPOSE_TEXT=$(cat <<'COMPOSE'
__COMPOSE__
COMPOSE
)
NB_COMPOSE_B64=$(printf '%s' "$COMPOSE_TEXT" | base64 | tr -d '\n')

__APPLY__
