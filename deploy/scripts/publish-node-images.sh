#!/usr/bin/env bash
# Put the images a joining node needs into the registry on this host (ADR 0048).
#
#   deploy/scripts/publish-node-images.sh [image ...]
#
# With no arguments it publishes the node package: the runner and the egress proxy. Any other
# argument is a local image (for example nestbox-claude-code:local) to publish under its name.
# The registry listens on 127.0.0.1:5000 on this host, which Docker already treats as insecure-
# allowed, so no daemon setting changes. Nodes pull the same images at 10.77.0.1:5000/<name>.
set -euo pipefail
cd "$(dirname "$0")/.."

REGISTRY="${NESTBOX_PUSH_REGISTRY:-localhost:5000}"

if [ "$#" -eq 0 ]; then
  docker compose build sandbox-runner egress-proxy
  project="${COMPOSE_PROJECT_NAME:-nestbox}"
  set -- "${project}-sandbox-runner=nestbox-sandbox-runner:latest" \
         "${project}-egress-proxy=nestbox-egress-proxy:latest"
fi

for item in "$@"; do
  source_image="${item%%=*}"
  target="${item#*=}"
  # A bare image name keeps its name and tag in the registry.
  [ "$source_image" = "$item" ] && target="$source_image"
  echo "==> $source_image -> $REGISTRY/$target"
  docker tag "$source_image" "$REGISTRY/$target"
  docker push "$REGISTRY/$target"
done

echo "Catalogue:"
curl -fsS "http://127.0.0.1:5000/v2/_catalog"
echo
