#!/bin/sh
# Nestbox node apply, the second half of a join. Run as root on a machine that `prepare.sh` has
# prepared, with the facts from the join answer in the environment:
#
#   NB_NODE_NAME=box-1 NB_OVERLAY_IP=10.77.1.2 NB_HUB_PUBLIC_KEY=... sh apply.sh
#
# It brings up the tunnel to the control plane, applies the egress lock, writes the node's
# secrets and its compose file, and starts the runner and the egress proxy. Every fact arrives as
# an `NB_*` variable: nothing is looked up here and nothing is asked, so the same script serves
# the join script the control plane hands out and a "connect by IP" run over SSH. Running it
# again with the same values is safe; it rewrites the files and restarts the containers.
set -eu

DIR=/opt/nestbox-node

step() { printf '\033[1m==> %s\033[0m\n' "$*"; }

[ "$(id -u)" = 0 ] || { echo "Run this as root (sudo)." >&2; exit 1; }

# Every value below is required: a default for any of them would be a guess. A node that came up
# with a wrong registry or overlay address is far harder to debug than a script that refused to
# start.
for name in NB_NODE_NAME NB_OVERLAY_IP NB_HUB_PUBLIC_KEY NB_HUB_IP NB_REGISTRY \
            NB_CONTROL_URL NB_NODE_TOKEN NB_NODE_ID NB_ENVIRONMENT_ID NB_EGRESS_LOCK \
            NB_COMPOSE_B64; do
  eval "value=\${$name-}"
  [ -n "$value" ] || {
    echo "apply.sh: $name is not set. It comes from the join answer of the control plane." >&2
    exit 1
  }
done

# Which side dials decides the shape of the tunnel, so exactly one of the two inputs is used.
LISTEN_PORT=${NB_LISTEN_PORT:-}
HUB_ENDPOINT=${NB_HUB_ENDPOINT:-}
if [ -n "$LISTEN_PORT" ]; then
  case "$LISTEN_PORT" in
    *[!0-9]*) echo "apply.sh: NB_LISTEN_PORT must be a number, not '$LISTEN_PORT'." >&2; exit 1 ;;
  esac
else
  [ -n "$HUB_ENDPOINT" ] || {
    echo "apply.sh: NB_HUB_ENDPOINT is empty and NB_LISTEN_PORT is not set; one of the two decides which side dials." >&2
    exit 1
  }
fi

case "$NB_EGRESS_LOCK" in
  strict|exit_local) ;;
  *) echo "apply.sh: NB_EGRESS_LOCK must be 'strict' or 'exit_local', not '$NB_EGRESS_LOCK'." >&2; exit 1 ;;
esac

[ -s /etc/wireguard/nb0.key ] || {
  echo "apply.sh: /etc/wireguard/nb0.key is missing. Run prepare.sh on this machine first." >&2
  exit 1
}

step "Bringing up the tunnel to the control plane"
mkdir -p "$DIR/secrets" /etc/wireguard /etc/nestbox
# The lock reads these back: the hub's address for a split tunnel, and the WAN interface for the
# kill switch. The default route is still the WAN here, so it is the right moment to note it.
printf '%s\n' "$NB_HUB_IP" > /etc/nestbox/hub-ip
if [ ! -s /etc/nestbox/wan ]; then
  wan="$(ip route show default | awk '$1 == "default" {print $5; exit}')"
  if [ -n "$wan" ]; then printf '%s\n' "$wan" > /etc/nestbox/wan; fi
fi

{
  printf '[Interface]\n'
  printf 'Address = %s/32\n' "$NB_OVERLAY_IP"
  printf 'PrivateKey = %s\n' "$(cat /etc/wireguard/nb0.key)"
  if [ -n "$LISTEN_PORT" ]; then
    printf 'ListenPort = %s\n' "$LISTEN_PORT"
  fi
  printf '\n[Peer]\n'
  printf 'PublicKey = %s\n' "$NB_HUB_PUBLIC_KEY"
  if [ -n "$LISTEN_PORT" ]; then
    # The hub dials this node, so the peer carries no Endpoint: the address is learned from the
    # first handshake that arrives. That is what makes a machine behind CGNAT or a rented box
    # with no reachable UDP port a node at all.
    printf 'AllowedIPs = %s/32\n' "$NB_HUB_IP"
  else
    printf 'Endpoint = %s\n' "$HUB_ENDPOINT"
    printf 'AllowedIPs = %s/32\n' "$NB_HUB_IP"
    printf 'PersistentKeepalive = 25\n'
  fi
} > /etc/wireguard/nb0.conf
chmod 600 /etc/wireguard/nb0.conf

if [ -n "$LISTEN_PORT" ] && command -v iptables >/dev/null 2>&1; then
  # A closed port is a node the hub can never dial: open the one port, once, and leave the rest
  # of the firewall to whoever set the machine up.
  iptables -C INPUT -p udp --dport "$LISTEN_PORT" -j ACCEPT 2>/dev/null \
    || iptables -I INPUT -p udp --dport "$LISTEN_PORT" -j ACCEPT
fi

systemctl enable wg-quick@nb0 >/dev/null 2>&1 || true
wg-quick down nb0 >/dev/null 2>&1 || true
wg-quick up nb0
# A node that dials the hub is reachable within a second. A hub-dialled node answers only after
# the hub has tried, so it waits longer.
WAIT=20
HINT="Is UDP ${HUB_ENDPOINT##*:} open between this machine and the hub?"
if [ -n "$LISTEN_PORT" ]; then
  WAIT=60
  HINT="Is UDP $LISTEN_PORT reachable from the hub, and is the port open on this machine?"
fi
for _ in $(seq 1 "$WAIT"); do ping -c1 -W1 "$NB_HUB_IP" >/dev/null 2>&1 && break; sleep 1; done
ping -c1 -W2 "$NB_HUB_IP" >/dev/null 2>&1 || {
  echo "The tunnel is up but $NB_HUB_IP does not answer. $HINT" >&2
  exit 1
}

step "Letting Docker pull from the control plane's registry ($NB_REGISTRY)"
CHANGED=$(python3 - "$NB_REGISTRY" <<'PY'
import json, os, sys
path = "/etc/docker/daemon.json"
conf = json.load(open(path)) if os.path.exists(path) and os.path.getsize(path) else {}
reg = conf.setdefault("insecure-registries", [])
if sys.argv[1] in reg:
    print("same")
else:
    reg.append(sys.argv[1])
    os.makedirs("/etc/docker", exist_ok=True)
    json.dump(conf, open(path, "w"), indent=2)
    print("changed")
PY
)
if [ "$CHANGED" = changed ]; then systemctl restart docker; sleep 3; fi

step "Writing the node's files"
printf '%s' "$NB_NODE_TOKEN" > "$DIR/secrets/node-token"
chmod 644 "$DIR/secrets/node-token"
mkdir -p "$DIR/secrets/proxy" "$DIR/state" /var/lib/nestbox/persistent /var/lib/nestbox/backups /var/lib/nestbox/egress
if [ ! -s "$DIR/secrets/proxy/egress-ca.key" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=Nestbox node CA ($NB_NODE_NAME)" \
    -keyout "$DIR/secrets/proxy/egress-ca.key" -out "$DIR/secrets/proxy/egress-ca.crt" >/dev/null 2>&1
fi
cp "$DIR/secrets/node-token" "$DIR/secrets/proxy/node-token"
chmod 644 "$DIR/secrets/proxy/"*
# The proxy and the runner run as their own users; they must be able to walk to these files.
# (The umask of prepare.sh keeps the WireGuard key private, which is the one file that must stay so.)
chmod 755 "$DIR" "$DIR/secrets" "$DIR/secrets/proxy"
cat > "$DIR/.env" <<ENV
REGISTRY=$NB_REGISTRY
OVERLAY_IP=$NB_OVERLAY_IP
HUB_IP=$NB_HUB_IP
CONTROL_OVERLAY=$NB_CONTROL_URL
CONTROL_HOST=$(python3 -c 'import sys,urllib.parse as u; p=u.urlparse(sys.argv[1]); print(p.hostname)' "$NB_CONTROL_URL")
CONTROL_PORT=$(python3 -c 'import sys,urllib.parse as u; p=u.urlparse(sys.argv[1]); print(p.port or 80)' "$NB_CONTROL_URL")
NODE_ID=$NB_NODE_ID
NODE_NAME=$NB_NODE_NAME
ENVIRONMENT_ID=$NB_ENVIRONMENT_ID
NODE_DIR=$DIR
ENV
printf '%s' "$NB_COMPOSE_B64" | base64 -d > "$DIR/docker-compose.yml"

step "Installing the egress lock"
cat > /usr/local/bin/nestbox-node-lock <<'LOCK'
#!/bin/sh
# nestbox-node-lock — set where this node's traffic leaves and what a sandbox can reach.
#
#   strict      the node is a full WireGuard tunnel: everything exits through the hub, and a
#               container is stopped from bypassing it on the WAN interface.
#   exit_local  the split tunnel of today: only the overlay goes through the hub, the internet
#               is reached directly from this machine.
#
# Written by the node's apply script; the sync timer runs it when the control plane asks for a
# different value. It is a script and not a long-running service because it changes one file and
# one iptables rule — and a half-applied lock is worse than a visible failure.
set -eu

want="${1-}"
conf=/etc/wireguard/nb0.conf

case "$want" in
  strict|exit_local) ;;
  *) echo "usage: nestbox-node-lock strict|exit_local" >&2; exit 2 ;;
esac

mkdir -p /etc/nestbox
# The hub's overlay address, remembered by the apply script; 10.77.0.1 is the hub's default.
hub_ip="$(cat /etc/nestbox/hub-ip 2>/dev/null || echo 10.77.0.1)"

# The WAN interface the kill switch covers. Once a full tunnel is up the default route is the
# tunnel itself, so the first non-tunnel answer is remembered and reused from then on.
wan="$(ip route show default | awk '$1 == "default" && $5 != "nb0" {print $5; exit}')"
if [ -n "$wan" ]; then
  printf '%s\n' "$wan" > /etc/nestbox/wan
else
  wan="$(cat /etc/nestbox/wan 2>/dev/null || true)"
fi

case "$want" in
  strict)
    # Full tunnel: everything leaves through the hub. wg-quick installs the fwmark rules, so its
    # own packets to the hub still go out of the WAN instead of back into the tunnel.
    sed -i 's|^AllowedIPs = .*|AllowedIPs = 0.0.0.0/0|' "$conf"
    grep -q '^DNS' "$conf" || sed -i '/^\[Interface\]/a DNS = 1.1.1.1' "$conf"
    wg-quick down nb0 2>/dev/null || true; wg-quick up nb0
    # Kill switch for containers: nothing forwarded out of the WAN interface. Host processes
    # (image pulls, model downloads) follow the default route, which is now the tunnel.
    if [ -n "$wan" ]; then
      if ! iptables -C DOCKER-USER -o "$wan" -j REJECT 2>/dev/null; then
        iptables -I DOCKER-USER -o "$wan" -j REJECT || {
          echo "nestbox-node-lock: cannot add the kill switch; is Docker running?" >&2
          exit 1
        }
      fi
    else
      echo "nestbox-node-lock: no WAN interface found; the container kill switch is not in place" >&2
    fi
    ;;
  exit_local)
    sed -i "s|^AllowedIPs = .*|AllowedIPs = ${hub_ip}/32|" "$conf"
    sed -i '/^DNS = /d' "$conf"
    wg-quick down nb0 2>/dev/null || true; wg-quick up nb0
    if [ -n "$wan" ]; then
      iptables -D DOCKER-USER -o "$wan" -j REJECT 2>/dev/null || true
    fi
    ;;
esac

echo "$want" > /etc/nestbox/egress-lock
echo "egress lock: $want (wan ${wan:-unknown})"
LOCK
chmod 755 /usr/local/bin/nestbox-node-lock

step "Installing the sync timer (the node follows the control plane's version and its lock)"
cat > /usr/local/bin/nestbox-node-sync <<'SYNC'
#!/bin/sh
# nestbox-node-sync — bring this node in step with its control plane: the compose file, the
# images, and the egress lock the control plane wants. Run by a timer; safe to run by hand.
set -eu
DIR=/opt/nestbox-node
cd "$DIR"
. ./.env

TMP=$(mktemp)
if curl -fsS -m 20 -H "Authorization: Bearer $(cat secrets/node-token)" -H "x-nestbox-node: $NODE_ID" \
     "$CONTROL_OVERLAY/api/v1/node/compose" -o "$TMP" && [ -s "$TMP" ]; then
  if ! cmp -s "$TMP" docker-compose.yml; then cp "$TMP" docker-compose.yml; echo "node package updated"; fi
fi
rm -f "$TMP"

# The lock the control plane wants, applied only when it differs from what is on this machine.
# A node that cannot reach the control plane keeps the lock it has, which is the safe direction.
LOCK_JSON=$(curl -fsS -m 20 -H "Authorization: Bearer $(cat secrets/node-token)" -H "x-nestbox-node: $NODE_ID" \
  "$CONTROL_OVERLAY/api/v1/node/egress-lock" 2>/dev/null || true)
WANT=$(printf '%s' "$LOCK_JSON" | python3 -c 'import json,sys
text = sys.stdin.read().strip()
print((json.loads(text).get("lock") or "") if text else "")' 2>/dev/null || true)
CURRENT=$(cat /etc/nestbox/egress-lock 2>/dev/null || true)
if [ -n "$WANT" ] && [ "$WANT" != "$CURRENT" ]; then
  nestbox-node-lock "$WANT" || echo "egress lock '$WANT' could not be applied" >&2
fi

docker compose --env-file .env pull -q
docker compose --env-file .env up -d
SYNC
chmod 755 /usr/local/bin/nestbox-node-sync
cat > /etc/systemd/system/nestbox-node-sync.service <<'UNIT'
[Unit]
Description=Bring this Nestbox node in step with its control plane
After=network-online.target docker.service wg-quick@nb0.service
[Service]
Type=oneshot
ExecStart=/usr/local/bin/nestbox-node-sync
UNIT
cat > /etc/systemd/system/nestbox-node-sync.timer <<'UNIT'
[Unit]
Description=Nestbox node sync every five minutes
[Timer]
OnBootSec=2min
OnUnitActiveSec=5min
[Install]
WantedBy=timers.target
UNIT
systemctl daemon-reload
systemctl enable --now nestbox-node-sync.timer >/dev/null 2>&1 || true

step "Applying the egress lock ($NB_EGRESS_LOCK)"
# Docker has to be running for the DOCKER-USER chain the kill switch lives in, and prepare.sh
# started it. This runs before the first pull on purpose: the images then come through the lock's
# tunnel instead of around it.
nestbox-node-lock "$NB_EGRESS_LOCK"

step "Pulling and starting the runner and the egress proxy"
cd "$DIR"
docker compose --env-file .env pull
docker compose --env-file .env up -d
for _ in $(seq 1 30); do curl -fsS "http://$NB_OVERLAY_IP:9000/healthz" >/dev/null 2>&1 && break; sleep 2; done
curl -fsS "http://$NB_OVERLAY_IP:9000/healthz" >/dev/null 2>&1 || {
  echo "The runner did not come up; see: docker compose -f $DIR/docker-compose.yml logs" >&2
  exit 1
}
step "Done. This machine is node '$NB_NODE_NAME'. It shows up online in the console within a minute."
