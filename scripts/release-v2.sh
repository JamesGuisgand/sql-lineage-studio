#!/usr/bin/env bash
# Replaces ONLY SQL Lineage Studio V1, without restarting Traefik or unrelated sites.
# Usage: bash scripts/release-v2.sh
set -Eeuo pipefail
V2_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
V1_DIR="${SQL_LINEAGE_V1_DIR:-$HOME/docker/portefolio/sql-lineage-web}"
EXPECTED_DOMAIN="rust-sql-projet.guisgand-james.fr"
PROJECT="sql-lineage-studio"
STATE_DIR="$V2_DIR/.release-state"
BACKUP_DIR="$HOME/docker/portefolio/sql-lineage-backups"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
V1_IMAGE="sql-lineage-studio:local"
BACKUP_IMAGE="sql-lineage-studio:v1-backup-$STAMP"
SWITCHING=0
SUCCESS=0

compose_v1() { docker compose --project-directory "$V1_DIR" -f "$V1_DIR/compose.yaml" "$@"; }
compose_v2() { docker compose --project-directory "$V2_DIR" -f "$V2_DIR/compose.yaml" "$@"; }

rollback_on_error() {
  local code=$?
  trap - ERR
  if [[ "$SWITCHING" == 1 && "$SUCCESS" == 0 ]]; then
    echo 'ERREUR pendant la bascule : tentative de restauration de la V1...' >&2
    docker image tag "$BACKUP_IMAGE" "$V1_IMAGE" || true
    compose_v1 up -d --no-build --force-recreate web || true
    echo 'Vérifie immédiatement : docker compose -f '"$V1_DIR/compose.yaml"' ps' >&2
  fi
  exit "$code"
}
trap rollback_on_error ERR

for bin in docker curl python3; do command -v "$bin" >/dev/null || { echo "Outil manquant : $bin" >&2; exit 1; }; done
docker compose version >/dev/null
[[ "$V2_DIR" != "$V1_DIR" ]] || { echo 'ERREUR : source V2 et V1 identiques.' >&2; exit 1; }
[[ -f "$V1_DIR/compose.yaml" && -f "$V1_DIR/.env" ]] || { echo "ERREUR : V1 ou .env introuvable dans $V1_DIR" >&2; exit 1; }
[[ -f "$V2_DIR/compose.yaml" && -f "$V2_DIR/Dockerfile" ]] || { echo 'ERREUR : V2 incomplète.' >&2; exit 1; }
[[ ! -e "$STATE_DIR/last-release.env" ]] || { echo 'Bascule déjà enregistrée : ne pas écraser la sauvegarde. Utilise rollback-v1.sh si nécessaire.' >&2; exit 1; }

# Transfer existing HTTPS settings, which are excluded from ZIP/Git.
if [[ ! -f "$V2_DIR/.env" ]]; then cp -p "$V1_DIR/.env" "$V2_DIR/.env"; fi
if ! grep -Fxq "DOMAIN=$EXPECTED_DOMAIN" "$V2_DIR/.env"; then
  echo "ERREUR : DOMAIN doit être $EXPECTED_DOMAIN dans $V2_DIR/.env" >&2; exit 1
fi
for check in 'TRAEFIK_NETWORK=web' 'TRAEFIK_ENTRYPOINT=websecure' 'TRAEFIK_CERTRESOLVER=myresolver'; do
  grep -Fxq "$check" "$V2_DIR/.env" || { echo "ERREUR : attendu dans le .env V2 : $check" >&2; exit 1; }
done
compose_v2 config --quiet
docker network inspect web >/dev/null
V1_CONTAINER="$(compose_v1 ps -q web)"
[[ -n "$V1_CONTAINER" ]] || { echo 'ERREUR : service V1 web introuvable. Aucune bascule.' >&2; exit 1; }
[[ "$(docker inspect -f '{{.State.Running}}' "$V1_CONTAINER")" == true ]] || { echo 'ERREUR : V1 non démarrée. Aucune bascule.' >&2; exit 1; }
docker image inspect "$V1_IMAGE" >/dev/null

# An earlier manual switch may already have put V2 online. Do not mislabel it as a V1 backup.
if curl -fsS --connect-timeout 3 --max-time 8 "https://$EXPECTED_DOMAIN/api/info" 2>/dev/null | \
  python3 -c 'import json,sys; o=json.load(sys.stdin); assert o.get("version") == "2.0.0"' 2>/dev/null; then
  echo 'La version 2.0.0 est déjà publique : bascule annulée pour éviter de remplacer une V2 existante.' >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR" "$STATE_DIR"
SOURCE_BACKUP="$BACKUP_DIR/sql-lineage-web-v1-$STAMP"
cp -a -- "$V1_DIR" "$SOURCE_BACKUP"
docker image tag "$V1_IMAGE" "$BACKUP_IMAGE"
cat >"$STATE_DIR/last-release.env" <<EOF
V1_DIR=$V1_DIR
V2_DIR=$V2_DIR
SOURCE_BACKUP=$SOURCE_BACKUP
BACKUP_IMAGE=$BACKUP_IMAGE
EOF
printf '\nSauvegarde : %s\nImage de rollback : %s\n' "$SOURCE_BACKUP" "$BACKUP_IMAGE"
printf '\n[1/3] Construction + tests Rust dans Docker (V1 reste active)...\n'
compose_v2 build web
printf '\n[2/3] Remplacement du seul conteneur SQL Lineage Studio...\n'
SWITCHING=1
compose_v2 up -d --no-build --force-recreate web
printf '\n[3/3] Vérification du HTTPS et de la version V2...\n'
HEALTHY=0
for i in $(seq 1 25); do
  if curl -fsS --connect-timeout 3 --max-time 7 "https://$EXPECTED_DOMAIN/api/info" 2>/dev/null | \
    python3 -c 'import sys,json; o=json.load(sys.stdin); assert o["version"] == "2.0.0"; assert "column_lineage" in o["features"]' 2>/dev/null; then
    HEALTHY=1
    break
  fi
  sleep 2
done
[[ "$HEALTHY" == 1 ]] || { echo 'La V2 ne répond pas correctement sur le domaine HTTPS.' >&2; false; }
SUCCESS=1
compose_v2 ps
printf '\nBascule réussie. V1 sauvegardée. Pour rollback : bash scripts/rollback-v1.sh\n'
printf 'Site : https://%s\n' "$EXPECTED_DOMAIN"
