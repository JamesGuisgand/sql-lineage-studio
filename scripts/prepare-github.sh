#!/usr/bin/env bash
# Normalize code and run local checks before first GitHub push.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || { echo 'Installe Rust/Cargo sur cette machine.' >&2; exit 1; }
command -v python3 >/dev/null || { echo 'Python 3 est requis.' >&2; exit 1; }
cargo fmt --manifest-path engine/Cargo.toml --all
cargo fmt --manifest-path engine/Cargo.toml --all -- --check
cargo test --manifest-path engine/Cargo.toml --all-targets
cargo clippy --manifest-path engine/Cargo.toml --all-targets
cargo build --release --manifest-path engine/Cargo.toml
SQL_LINEAGE_REAL_BIN="$PWD/engine/target/release/sql-lineage-rs" python3 -m unittest discover -s tests -v
if command -v node >/dev/null; then node --check web/app.js; else echo 'Node.js absent : le test JS sera exécuté sur GitHub Actions.'; fi
echo 'Vérifications locales terminées. Tu peux créer le commit GitHub.'
