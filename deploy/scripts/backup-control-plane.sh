#!/usr/bin/env bash
# Back up the control plane itself (ADR 0049): the PostgreSQL database and, separately, the key
# that opens every stored secret.
#
#   deploy/scripts/backup-control-plane.sh [destination-dir]
#
# Writes <dir>/nestbox-db-<time>.sql.gz and, if the vault key exists, <dir>/vault_key-<time>
# (mode 0400). Keep the two in different places: the database without the key holds secrets
# nobody can open, and the key next to the database defeats the point of encrypting them.
# Restore: `gunzip -c file | docker compose exec -T postgres psql -U nestbox nestbox`.
set -euo pipefail
cd "$(dirname "$0")/.."

dest="${1:-./backups}"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$dest"
umask 077

docker compose exec -T postgres pg_dump -U nestbox --no-owner nestbox | gzip -9 > "$dest/nestbox-db-$stamp.sql.gz"
echo "database  -> $dest/nestbox-db-$stamp.sql.gz ($(du -h "$dest/nestbox-db-$stamp.sql.gz" | cut -f1))"

if [ -f secrets/vault_key ]; then
  cp secrets/vault_key "$dest/vault_key-$stamp"
  chmod 0400 "$dest/vault_key-$stamp"
  echo "vault key -> $dest/vault_key-$stamp  (store it somewhere else)"
fi
