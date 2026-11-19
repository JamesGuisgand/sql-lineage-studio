#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
echo "[1/4] Compilation, tests Rust, image Docker isolée"
docker compose -f compose.test.yaml up -d --build
for n in {1..30}; do
  if curl -fsS http://127.0.0.1:18080/healthz >/dev/null; then break; fi
  sleep 2
done
echo "[2/4] Santé serveur"
curl -fsS http://127.0.0.1:18080/healthz
echo
echo "[3/4] Analyse réelle de colonnes + impact (un appel Rust)"
curl -fsS -H 'Content-Type: application/json' -X POST \
  --data-binary '{"sql":"CREATE VIEW b AS SELECT a.id FROM a; CREATE VIEW c AS SELECT b.id FROM b;","impact_table":"a"}' \
  http://127.0.0.1:18080/api/analyze | python3 -c '
import json,sys
x=json.load(sys.stdin)
assert len(x["statements"]) == 2
assert x["impact"]["affected"] == ["b","c"], x
assert len(x["columns"]) >= 2, x
print("OK : 2 instructions, lineage colonnes, impact transitif a -> b -> c")'
echo "[4/4] Test terminé : production inchangée"
echo "Pour accéder depuis ton PC: ssh -L 18080:127.0.0.1:18080 james@IP_DU_VPS"
echo "Puis ouvre http://127.0.0.1:18080"
