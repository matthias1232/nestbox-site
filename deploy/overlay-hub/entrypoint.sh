#!/bin/sh
# Hub for the node overlay. Files in /overlay:
#   hub.key    the hub's private key, made once, 0600
#   hub.pub    its public key, read by the control plane and given to joining nodes
#   peers.conf the nodes, written by the control plane; applied here within a few seconds
#   status.tsv `wg show nb-hub dump`, rewritten every 3 s for the control API
set -eu

DIR=/overlay
IFACE=nb-hub
HUB_IP="${NESTBOX_OVERLAY_HUB_IP:-10.77.0.1}"
PORT="${NESTBOX_OVERLAY_PORT:-51821}"
API_UID="${NESTBOX_API_UID:-10001}"

mkdir -p "$DIR"
umask 077
[ -s "$DIR/hub.key" ] || wg genkey > "$DIR/hub.key"
wg pubkey < "$DIR/hub.key" > "$DIR/hub.pub.new"
chmod 644 "$DIR/hub.pub.new"
mv "$DIR/hub.pub.new" "$DIR/hub.pub"
# The control plane writes peers.conf here as its own user; the key stays root-only.
chown "$API_UID:$API_UID" "$DIR"
chmod 755 "$DIR"
[ -f "$DIR/peers.conf" ] || { : > "$DIR/peers.conf"; chown "$API_UID:$API_UID" "$DIR/peers.conf"; chmod 644 "$DIR/peers.conf"; }

ip link del "$IFACE" 2>/dev/null || true
ip link add "$IFACE" type wireguard
ip addr add "$HUB_IP/16" dev "$IFACE"
wg set "$IFACE" listen-port "$PORT" private-key "$DIR/hub.key"
ip link set "$IFACE" up

# Nodes of different tenants share this interface and must not reach each other: nothing
# is forwarded from the overlay back into the overlay.
iptables -C FORWARD -i "$IFACE" -o "$IFACE" -j DROP 2>/dev/null || iptables -I FORWARD 1 -i "$IFACE" -o "$IFACE" -j DROP
# The control plane's containers reach a node through the host. Masquerade gives their packets
# the hub's address, which is the only address a node accepts.
iptables -t nat -C POSTROUTING -o "$IFACE" -j MASQUERADE 2>/dev/null || iptables -t nat -A POSTROUTING -o "$IFACE" -j MASQUERADE
# A node with the strict egress lock keeps its local exit shut and sends everything into the
# overlay, so the hub is the only way to the internet. Without these rules such a node reaches
# the hub's services but nothing beyond it (ADR 0057). The overlay-to-overlay DROP above stays
# rule #1: forwarding is allowed only between the overlay and whatever the hub itself uses.
SUBNET="${HUB_IP%.*}.0/16"
sysctl -w net.ipv4.ip_forward=1 >/dev/null
iptables -C FORWARD -i "$IFACE" ! -o "$IFACE" -j ACCEPT 2>/dev/null || iptables -I FORWARD 2 -i "$IFACE" ! -o "$IFACE" -j ACCEPT
iptables -C FORWARD -o "$IFACE" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || iptables -I FORWARD 2 -o "$IFACE" -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -t nat -C POSTROUTING -s "$SUBNET" ! -o "$IFACE" -j MASQUERADE 2>/dev/null || iptables -t nat -A POSTROUTING -s "$SUBNET" ! -o "$IFACE" -j MASQUERADE
# Allow the overlay's UDP port in, in case the host filters it.
iptables -C INPUT -p udp --dport "$PORT" -j ACCEPT 2>/dev/null || iptables -I INPUT 1 -p udp --dport "$PORT" -j ACCEPT
# A node reaches the control API (8000), the registry (5000), the snapshot server (8500) and the
# appliance's shared services (egress proxy 18087, MCP hub 18088, media renderer 18089, accounting
# proxy 18099) at the hub address only. Port
# 8000 is the API itself, not the console's nginx: the egress proxy asks it about CONNECT requests,
# which nginx refuses.
iptables -C INPUT -i "$IFACE" -p tcp -m multiport --dports 8000,5000,8500,18087,18088,18089,18099 -j ACCEPT 2>/dev/null \
  || iptables -I INPUT 1 -i "$IFACE" -p tcp -m multiport --dports 8000,5000,8500,18087,18088,18089,18099 -j ACCEPT

# The registry listens on the host's loopback (Docker treats that as insecure-allowed, so the
# host can push without a daemon change); nodes pull it at the hub address.
socat "TCP-LISTEN:5000,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:5000 &
socat "TCP-LISTEN:8000,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:18000 &
# The shared services of the appliance keep their loopback ports and are handed to joined nodes
# under the same numbers, so a node uses the addresses a sandbox uses on this machine (ADR 0057):
# egress proxy, MCP hub, media renderer and accounting proxy.
socat "TCP-LISTEN:18087,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:18087 &
socat "TCP-LISTEN:18088,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:18088 &
socat "TCP-LISTEN:18089,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:18089 &
socat "TCP-LISTEN:18099,bind=$HUB_IP,fork,reuseaddr" TCP:127.0.0.1:18099 &

last=""
while true; do
  if [ -f "$DIR/peers.conf" ]; then
    sum=$(md5sum < "$DIR/peers.conf")
    if [ "$sum" != "$last" ]; then
      {
        printf '[Interface]\nPrivateKey = %s\nListenPort = %s\n' "$(cat "$DIR/hub.key")" "$PORT"
        cat "$DIR/peers.conf"
      } > /tmp/wg.conf
      if wg syncconf "$IFACE" /tmp/wg.conf; then last="$sum"; echo "applied $(grep -c '^\[Peer\]' "$DIR/peers.conf") peer(s)"; fi
    fi
  fi
  # The control API reads this dump to report each tunnel's handshake age and byte counters
  # without asking the hub. The write goes through a temporary file so a reader never sees half
  # a line; the file is world-readable because the API runs as its own user (ADR 0057).
  wg show "$IFACE" dump > "$DIR/status.tsv.tmp" 2>/dev/null && chmod 644 "$DIR/status.tsv.tmp" && mv "$DIR/status.tsv.tmp" "$DIR/status.tsv"
  sleep 3
done
