#!/usr/bin/env bash
# Copy this node's egress CA into every agent image context.
#
# A TLS-terminating egress proxy presents a certificate signed by this node's
# CA. An image that does not trust it turns every HTTPS request into a TLS
# error, which looks like a broken sandbox rather than a missing root. The CA is
# generated per deployment (`make-egress-ca.sh`) and is therefore not committed;
# the agent Dockerfiles copy the file from their own context, so it has to be
# placed there before the image is built.
#
# Run this after `make-egress-ca.sh` and before `docker compose build`.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
deploy_dir="$(dirname "$here")"
source_ca="$deploy_dir/secrets/egress/ca.crt"

if [ ! -s "$source_ca" ]; then
  echo "no egress CA at $source_ca; run make-egress-ca.sh first" >&2
  exit 1
fi

count=0
for dir in "$deploy_dir"/../services/tool-images/*/; do
  [ -f "$dir/Dockerfile" ] || continue
  grep -q 'nestbox-egress' "$dir/Dockerfile" || continue
  install -m 0644 "$source_ca" "$dir/egress-ca.crt"
  count=$((count + 1))
  echo "  $(basename "$dir")"
done

if [ "$count" -eq 0 ]; then
  echo "no agent Dockerfile installs the egress CA; nothing copied" >&2
  exit 1
fi
echo "copied the egress CA into $count image contexts"
