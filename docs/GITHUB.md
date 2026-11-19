# Publier SQL Lineage Studio V2 sur GitHub

## 1. Préparer les tests et le formatage

Sur le VPS, dans **le dossier V2 complet** :

```bash
cd ~/sql-lineage-studio-v2-release
rustup component add rustfmt clippy
bash scripts/prepare-github.sh
```

`cargo fmt` reformate le code Rust. C'est nécessaire avant le premier push, car GitHub Actions utilise `cargo fmt --check`.

## 2. Créer le dépôt

Créer un dépôt public vide sur https://github.com/new :

- Compte : `JamesGuisgand`.
- Nom : `sql-lineage-studio`.
- Ne pas cocher README, licence ou `.gitignore` (ils existent déjà dans le paquet).

## 3. Authentifier GitHub depuis le VPS

```bash
sudo apt update
sudo apt install -y gh
gh auth login
```

Choisir `GitHub.com`, `HTTPS`, puis **Login with a web browser** et effectuer l'authentification avec le navigateur. `gh` configure l'authentification Git à l'aide de l'identité autorisée.

## 4. Envoyer les sources

```bash
cd ~/sql-lineage-studio-v2-release
git init -b main
git add .
git status --short
git commit -m "Release SQL Lineage Studio v2.0.0"
git remote add origin https://github.com/JamesGuisgand/sql-lineage-studio.git
git push -u origin main
```

Si Git demande une identité, définir `git config user.name` et `git config user.email` avec les informations GitHub du compte et recommencer le commit.

**Avant de pousser :** vérifier que `git status --short` ne contient **pas** `.env`, `.release-state`, des clés SSH, des secrets ni `acme.json`.

## 5. Vérifier la CI

Ouvrir : https://github.com/JamesGuisgand/sql-lineage-studio/actions

Le workflow `CI · SQL Lineage Studio V2` se lance sur chaque push et pull request. Il exécute 3 jobs : Rust, API+frontend, image Docker. Si un job échoue, cliquer sur son nom pour voir la commande et les logs.

## 6. Renseigner About

- **Description** : `Rust AST-based SQL lineage analyzer with column dependencies, impact analysis, interactive graphs and Docker deployment.`
- **Website** : `https://rust-sql-projet.guisgand-james.fr`
- **Topics** : `rust`, `sql`, `data-lineage`, `data-engineering`, `graph`, `docker`, `sql-parser`.

Le lien GitHub dans l'interface V2 pointe déjà vers ce nouveau dépôt. Il sera actif après le push.

## Important : pas de CD automatique

Les tests sont automatiques, **mais aucun secret GitHub, accès SSH VPS ni redéploiement automatique ne sont créés**. Aucun commit ne doit modifier la production sans une action de déploiement séparée.
