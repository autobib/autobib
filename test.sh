#!/usr/bin/env bash
set -euo pipefail

REMOTES_FILE="tests/remotes.txt"
HASH_REMOTES="$(cat $REMOTES_FILE src/http/cache.rs | sha256)"
HASH_SRC="$(cat src/http/cache.rs | sha256)"
CACHE_DIR="target/tmp/test-cache-$HASH_REMOTES-$HASH_SRC"

export AUTOBIB_RESPONSE_CACHE_PATH="$CACHE_DIR/responses.dat"

# set database file, but immediately delete it if it already exists
export AUTOBIB_DATABASE_PATH="$CACHE_DIR/records.db"
rm -f "$AUTOBIB_DATABASE_PATH"

if [ ! -f "$AUTOBIB_RESPONSE_CACHE_PATH=" ]; then
    echo "Cache file not found. Generating cache file: $AUTOBIB_RESPONSE_CACHE_PATH"
    
    mkdir -p "$(dirname "$AUTOBIB_RESPONSE_CACHE_PATH")"

    # Generate the cache file
    cat $REMOTES_FILE | xargs -- cargo run --features write_response_cache -- -vv get --retrieve-only --ignore-null
else
    echo "Cache file found: $AUTOBIB_RESPONSE_CACHE_PATH"
fi

cargo test --features read_response_cache
