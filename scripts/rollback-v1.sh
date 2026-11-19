#!/usr/bin/env bash
# Restore the V1 image and configuration saved by scripts/release-v2.sh.
set -Eeuo pipefail
V2_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE="$V2_DIR/.release-state/last-release.env"
[[ -f "$STATE" ]] || { echo 'Aucune release V2 enregistrée pour rollback.' >&2; exit 1; }
# The file is generated locally by release-v2.sh (not from user input).
source "$STATE"
[[ -f "$V1_DIR/compose.yaml" && -f "$V1_DIR/.env" ]] || { echo 'Dossier V1 introuvable.' >&2; exit 1; }
[[ -n "${BACKUP_IMAGE:-}" ]] || { echo 'Image de sauvegarde manquante.' >&2; exit 1; }
docker image inspect "$BACKUP_IMAGE" >/dev/null
# The V1 Compose config points to sql-lineage-studio:local.
docker image tag "$BACKUP_IMAGE" sql-lineage-studio:local
docker compose --project-directory "$V1_DIR" -f "$V1_DIR/compose.yaml" up -d --no-build --force-recreate web
printf '\nV1 restaurée. Vérification :\n'
docker compose --project-directory "$V1_DIR" -f "$V1_DIR/compose.yaml" ps
curl -fsSI --max-time 12 'https://rust-sql-projet.guisgand-james.fr' || true
printf '\nCode V1 sauvegardé : %s\n' "$SOURCE_BACKUP"
