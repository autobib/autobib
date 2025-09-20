#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cargo locate-project --message-format plain)"
cd "$(dirname "${PROJECT_ROOT}")"

REMOTES_FILE="tests/remotes.txt"
CACHE_FORMAT_DEF="src/http/cache/format.rs"
CACHE_HOME="${XDG_CACHE_HOME:=${HOME}/.cache/}"
CACHE_ROOT="${AUTOBIB_RESPONSE_CACHE_DIR:=${CACHE_HOME}/autobib}"

# set up cache directory
mkdir -p "${CACHE_ROOT}"
HASH="$(cat "${REMOTES_FILE}" "${CACHE_FORMAT_DEF}" | sha256)"
CACHE_DIR="${CACHE_ROOT}/test-cache-${HASH}"

export AUTOBIB_RESPONSE_CACHE_PATH="${CACHE_DIR}/responses.dat"

if [[ ! -f "${AUTOBIB_RESPONSE_CACHE_PATH}" ]]; then
    echo "Cache file not found. Generating cache file: ${AUTOBIB_RESPONSE_CACHE_PATH}"
    
    mkdir -p "${CACHE_DIR}"

    # Generate the cache file
    xargs -- cargo run --locked --features write_response_cache -- -vv get --retrieve-only --ignore-null < "${REMOTES_FILE}"
else
    echo "Cache file found: ${AUTOBIB_RESPONSE_CACHE_PATH}"
fi

cargo test --locked --no-fail-fast --features read_response_cache

cargo doc --no-deps --locked
cargo clippy --locked
cargo fmt --check
sort -C "${REMOTES_FILE}"
REPETITIONS="$(uniq -d < "${REMOTES_FILE}" | wc -w)"
test "${REPETITIONS}" -eq 0
shellcheck scripts/*.sh --enable=all
