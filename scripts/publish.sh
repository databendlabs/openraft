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

INDEX_POLL_INTERVAL=3
INDEX_TIMEOUT=180

# crates.io rejects API requests without a descriptive User-Agent (403),
# per https://crates.io/data-access.
USER_AGENT="openraft-publish-script (https://github.com/databendlabs/openraft)"

crate_version_exists() {
    local url="https://crates.io/api/v1/crates/$1/$2"
    local http_code
    http_code=$(curl -s -o /dev/null -w "%{http_code}" -A "$USER_AGENT" "$url")
    echo "DEBUG: GET $url -> HTTP $http_code" >&2
    [ "$http_code" = "200" ]
}

for crate in "${CRATES[@]}"; do
    manifest="$ROOT_DIR/$crate/Cargo.toml"
    name=$(grep -m1 '^name' "$manifest" | sed 's/name *= *"\(.*\)"/\1/')
    version=$(cargo metadata --manifest-path "$manifest" --no-deps --format-version 1 \
        | python3 -c "import sys,json; pkgs=json.load(sys.stdin)['packages']; print([p['version'] for p in pkgs if p['name']=='$name'][0])")

    echo ""
    echo "--- $name $version ($crate/) ---"

    # Check if this version already exists on crates.io
    if crate_version_exists "$name" "$version"; then
        echo "Already published, skipping."
        continue
    fi

    publish_ok=1
    publish_output=$(cargo publish --manifest-path "$manifest" $DRY_RUN 2>&1) || publish_ok=0
    echo "$publish_output"

    if [ "$publish_ok" -eq 0 ]; then
        if ! echo "$publish_output" | grep -q "already exists on crates.io index"; then
            exit 1
        fi
        echo "Already published, continuing."
    fi

    if [ -z "$DRY_RUN" ]; then
        echo "Waiting for crates.io to index $name $version..."
        elapsed=0
        until crate_version_exists "$name" "$version"; do
            if [ "$elapsed" -ge "$INDEX_TIMEOUT" ]; then
                echo "Error: $name $version not indexed after ${INDEX_TIMEOUT}s" >&2
                exit 1
            fi
            sleep "$INDEX_POLL_INTERVAL"
            elapsed=$((elapsed + INDEX_POLL_INTERVAL))
        done
    fi
done

echo ""
echo "=== All crates published ==="
