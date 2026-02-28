#!/bin/bash

set -euo pipefail

if [ -z "${CARGO_REGISTRY_TOKEN:-}" ]; then
    echo "Error: CARGO_REGISTRY_TOKEN is not set"
    echo "Usage: CARGO_REGISTRY_TOKEN=<token> $0 [--dry-run]"
    exit 1
fi

DRY_RUN=""
if [ "${1:-}" = "--dry-run" ]; then
    DRY_RUN="--dry-run"
    echo "=== DRY RUN ==="
fi

# Publish order based on dependency graph:
#   macros -> rt -> rt-tokio, rt-compio, rt-monoio -> openraft -> legacy, multiraft, memstore
CRATES=(
    macros
    rt
    rt-tokio
    rt-compio
    rt-monoio
    openraft
    legacy
    multiraft
    stores/memstore
    metrics-otel
)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

for crate in "${CRATES[@]}"; do
    manifest="$ROOT_DIR/$crate/Cargo.toml"
    name=$(grep -m1 '^name' "$manifest" | sed 's/name *= *"\(.*\)"/\1/')
    version=$(cargo metadata --manifest-path "$manifest" --no-deps --format-version 1 \
        | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; print([p['version'] for p in pkgs if p['name']=='$name'][0])")

    echo ""
    echo "--- Publishing $name $version ($crate/) ---"

    cargo publish --manifest-path "$manifest" $DRY_RUN

    if [ -z "$DRY_RUN" ]; then
        echo "Waiting for crates.io to index $name $version..."
        sleep 30
    fi
done

echo ""
echo "=== All crates published ==="
