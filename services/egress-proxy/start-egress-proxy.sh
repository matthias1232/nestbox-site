#!/bin/sh
# Prepare the configuration, then become Envoy.
#
# Envoy asks the control API for every decision, and the control API only answers
# a node that proves who it is. The node id and token are not in the image (a
# token in an image is a token in a registry): they are written into a copy of the
# configuration here, from the file the compose file mounts, and the copy lives in
# /tmp, readable only by this user.
#
# Run without the token (for example `docker compose run --entrypoint envoy` to
# validate the image) this script is bypassed and the image's own configuration is
# used.

set -eu

CONF=/etc/envoy/envoy.yaml
NODE_ID="${NESTBOX_NODE_ID:-}"
TOKEN=""
TOKEN_FILE="${NESTBOX_NODE_TOKEN_FILE:-/run/node/node-token}"
if [ -r "$TOKEN_FILE" ]; then
    TOKEN="$(tr -d '\r\n ' < "$TOKEN_FILE")"
fi
if [ -z "$NODE_ID" ] || [ -z "$TOKEN" ]; then
    echo "egress-proxy: no node id or token; the control API will refuse every check and every request is denied" >&2
fi
umask 0077
# `|` as the delimiter: neither value can contain it (a uuid and a hex token).
sed -e "s|__NESTBOX_NODE_ID__|${NODE_ID}|g" -e "s|__NESTBOX_NODE_TOKEN__|${TOKEN}|g" \
    "$CONF" > /tmp/envoy.yaml
CONF=/tmp/envoy.yaml
# The Lua filter asks the control API about tunnels, so it needs the same identity.
sed -e "s|__NESTBOX_NODE_ID__|${NODE_ID}|g" -e "s|__NESTBOX_NODE_TOKEN__|${TOKEN}|g" \
    /etc/envoy/extensions/mcp.lua > /tmp/mcp.lua
cp /etc/envoy/extensions/route.lua /tmp/route.lua

# The certificate broker is only useful for the TLS-reading mode; when its CA
# secret is absent it exits with a message and Envoy carries on, because the
# default HTTPS path (a decided tunnel) does not need it.
#
# It also checks the bodies it forwards and asks the control API with this
# node's identity: no second credential, and the same trust the tunnel check
# already relies on.
NESTBOX_NODE_ID="$NODE_ID" NESTBOX_NODE_TOKEN="$TOKEN" /usr/local/bin/egress-mitm &
broker=$!
sleep 1
if ! kill -0 "$broker" 2>/dev/null; then
    echo "egress-proxy: the certificate broker is not running; TLS is decided by destination, not read" >&2
fi

exec envoy -c "$CONF" --log-level "${ENVOY_LOG_LEVEL:-info}"
