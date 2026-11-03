#!/usr/bin/env bash
set -euo pipefail

# Apply the pushed branch to the dev appliance.
#
# The running stack lives in a git checkout (`/opt/nestbox/site`), so a deploy is
# "push, then fast-forward that checkout and rebuild", not a file copy. An rsync
# with --delete into another directory once removed files from the box; a
# fast-forward cannot delete anything the branch does not itself remove, and it
# refuses (instead of overwriting) when the checkout has diverged.
#
# A build needs free disk. The builder cache once filled the 116 GB root to
# 100 %, and Postgres stopped writing while the login answered 500. The guard
# below refuses to start a build below 15 GB, and the prune afterwards drops
# cache older than a day so it cannot grow into that state again.
#
#   git push origin HEAD            # first: the box pulls, it does not receive
#   deploy/scripts/deploy-appliance.sh
HOST=${NESTBOX_HOST:-192.168.55.50}
USER=${NESTBOX_USER:-nestbox}
KEY=${NESTBOX_KEY:-$HOME/.ssh/nestbox_builder}
SITE=${NESTBOX_SITE:-/opt/nestbox/site}
BRANCH=${NESTBOX_BRANCH:-$(git rev-parse --abbrev-ref HEAD)}

# The heredoc is quoted, so every `$` in it belongs to the box; the two values
# it needs arrive as positional arguments.
ssh -i "$KEY" "$USER@$HOST" "bash -s -- '$SITE' '$BRANCH'" <<'REMOTE'
set -euo pipefail
site=$1
branch=$2
free_gb=$(df -BG --output=avail / | tail -1 | tr -dc '0-9')
if [ "$free_gb" -lt 15 ]; then
  echo "Only ${free_gb} GB free on /; run: docker builder prune -af" >&2
  exit 1
fi
cd "$site" && git fetch origin "$branch" && git merge --ff-only FETCH_HEAD
cd deploy && docker compose up -d --build
docker builder prune -af --filter until=24h >/dev/null 2>&1 || true
REMOTE
