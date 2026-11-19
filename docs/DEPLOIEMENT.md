# Bascule V1 → V2 (VPS de production)

Cette procédure s'adresse à l'installation réelle `james@85.31.236.241` :

- V1 : `~/docker/portefolio/sql-lineage-web`.
- V2 : `~/sql-lineage-studio-v2-release` (ou dossier extrait portant ce nom).
- Traefik existant : réseau `web`, entrypoint `websecure`, certificat `myresolver`.
- Domaine : `https://rust-sql-projet.guisgand-james.fr`.

## Garanties du script

`bash scripts/release-v2.sh` sauvegarde la **V1** (sources et image Docker) avant de commencer, construit et teste la V2 **pendant que la V1 tourne encore**, puis recrée uniquement le conteneur du service `web` appartenant au projet Docker Compose `sql-lineage-studio`. Il ne redémarre **ni Docker, ni Traefik, ni aucun autre site**.

La bascule n'est **pas un zéro-downtime strict** : quelques secondes d'indisponibilité sont possibles. Si le HTTPS ne répond pas avec `version=2.0.0`, le script tente de restaurer la V1 automatiquement. Le dossier et l'image de sauvegarde sont conservés.

Ne **supprime pas** `~/docker/portefolio/sql-lineage-web`, même après la bascule : il sert au retour arrière. Ne lance pas `docker compose down` sans préciser le dossier ciblé.

## 1 — Valider le conteneur de test avant production

```bash
cd ~/sql-lineage-studio-v2-release
bash validate-v2.sh
```

## 2 — Basculer en production

```bash
cd ~/sql-lineage-studio-v2-release
bash scripts/release-v2.sh
```

La configuration `.env` est copiée de la V1 si elle n'existe pas dans le dossier V2. Dans tous les cas, le script contrôle les valeurs Traefik attendues.

## 3 — Vérifier

```bash
curl -fsS https://rust-sql-projet.guisgand-james.fr/api/info
curl -I https://rust-sql-projet.guisgand-james.fr
cd ~/sql-lineage-studio-v2-release
docker compose ps
```

Réponse attendue de `/api/info` : version `2.0.0`, fonctionnalités `ast`, `column_lineage`, `impact`, `dialects`.

## 4 — Arrêter seulement la préproduction locale

Une fois la V2 publique validée :

```bash
cd ~/sql-lineage-studio-v2-release
docker compose -f compose.test.yaml down
```

## 5 — Revenir à la V1

```bash
cd ~/sql-lineage-studio-v2-release
bash scripts/rollback-v1.sh
```

Le rollback réattribue à `sql-lineage-studio:local` l'image V1 sauvegardée, puis recrée le service web avec la configuration Compose V1. Il ne modifie ni Traefik ni les autres applications.

## Diagnostic

```bash
cd ~/sql-lineage-studio-v2-release
docker compose ps
docker compose logs web --tail=80
```

Ne jamais commiter ou envoyer le fichier `.env`, une clé SSH, un token, ou un fichier ACME `acme.json` dans GitHub.
