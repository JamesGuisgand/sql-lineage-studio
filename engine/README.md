# SQL Lineage Rust — V2

Moteur d'analyse statique de dépendances SQL basé sur le parseur AST `sqlparser-rs` 0.46.0.

```bash
cargo test
cargo run -- --format json --dialect generic examples/demo.sql
cargo run -- --format json --impact crm.customers examples/demo.sql
cargo run -- --format dot examples/demo.sql
```

Le moteur génère :

- `statements` : types d'instructions, cibles et sources physiques
- `columns` : colonnes sources et cibles lorsque leurs origines sont résolues
- `warnings` : cas ambigus, notamment `SELECT *` ou jointures sans qualification
- `impact` : descendants transitifs d'une table et chemins minimaux (sur demande)
- `dot` : représentation Graphviz du graphe de tables

Il n'ouvre aucune connexion à une base de données et n'exécute jamais le SQL fourni.

Voir [README principal](../README.md) et [limites](../docs/limits.md).
