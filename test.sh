#!/usr/bin/env bash
set -euo pipefail

REMOTES_FILE="tests/remotes.txt"

# set up cache directory
CACHE_ROOT="cache"
mkdir -p $CACHE_ROOT
touch $CACHE_ROOT/CACHEDIR.TAG
HASH="$(cat $REMOTES_FILE src/http/cache.rs | sha256)"
CACHE_DIR="$CACHE_ROOT/test-cache-$HASH"


export AUTOBIB_RESPONSE_CACHE_PATH="$CACHE_DIR/responses.dat"

# temporary database file location: delete it if it already exists
export AUTOBIB_DATABASE_PATH="$CACHE_ROOT/records.db"
rm -f "$AUTOBIB_DATABASE_PATH"

if [ ! -f "$AUTOBIB_RESPONSE_CACHE_PATH" ]; then
    echo "Cache file not found. Generating cache file: $AUTOBIB_RESPONSE_CACHE_PATH"
    
    mkdir -p "$(dirname "$AUTOBIB_RESPONSE_CACHE_PATH")"

    # Generate the cache file
    cat "$REMOTES_FILE" | xargs -- cargo run --locked --features write_response_cache -- -vv get --retrieve-only --ignore-null
else
    echo "Cache file found: $AUTOBIB_RESPONSE_CACHE_PATH"
fi

cargo test --locked --no-fail-fast --features read_response_cache

cargo doc --no-deps --locked
cargo clippy --locked
cargo fmt --check
sort -C "$REMOTES_FILE"
test "$(cat $REMOTES_FILE | uniq -d | wc -w)" -eq 0
