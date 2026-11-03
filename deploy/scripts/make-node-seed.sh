#!/usr/bin/env bash
# Write a NoCloud seed ISO that makes a fresh Ubuntu cloud image join Nestbox as a node on first boot.
#
#   make-node-seed.sh <control-plane-url> <join-token> [output.iso]
#
# Get the token under Nodes -> Add a node (the part between /join/ and /script). Attach the ISO as a
# CD-ROM to any VM started from an Ubuntu cloud image; no installer, no keyboard. The token works
# once for 15 minutes, so make the ISO shortly before booting.
set -euo pipefail

url="${1:?control plane URL, for example http://203.0.113.7:8080}"
token="${2:?join token}"
out="${3:-nestbox-node-seed.iso}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL "${url%/}/api/v1/join/${token}/cloud-init" -o "$work/user-data"
printf 'instance-id: nestbox-node-%s\nlocal-hostname: nestbox-node\n' "$(date +%s)" > "$work/meta-data"

if command -v cloud-localds >/dev/null 2>&1; then
  cloud-localds "$out" "$work/user-data" "$work/meta-data"
elif command -v xorriso >/dev/null 2>&1; then
  xorriso -as mkisofs -quiet -output "$out" -volid cidata -joliet -rock "$work/user-data" "$work/meta-data"
elif command -v genisoimage >/dev/null 2>&1; then
  genisoimage -quiet -output "$out" -volid cidata -joliet -rock "$work/user-data" "$work/meta-data"
else
  echo "Install cloud-image-utils, xorriso or genisoimage first." >&2
  exit 1
fi
echo "Wrote $out. Attach it to a VM booted from an Ubuntu cloud image."
