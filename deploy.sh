#!/usr/bin/env bash
# Deploy from this package WITH V1 backup and rollback support.
set -euo pipefail
cd "$(dirname "$0")"
exec bash scripts/release-v2.sh
