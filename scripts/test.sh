#!/usr/bin/env bash
set -euo pipefail

export RUSTFLAGS="-D warnings"
export RUSTDOCFLAGS="-D warnings"

PROJECT_ROOT="$(cargo locate-project --message-format plain)"
cd "$(dirname "${PROJECT_ROOT}")"

REMOTES_FILE="tests/remotes.txt"
CACHE_FORMAT_DEF="src/http/cache/format.rs"
CACHE_HOME="${XDG_CACHE_HOME:=${HOME}/.cache}"
CACHE_ROOT="${AUTOBIB_RESPONSE_CACHE_DIR:=${CACHE_HOME}/autobib}"

# set up cache directory
mkdir -p "${CACHE_ROOT}"
HASH="$(cat "${REMOTES_FILE}" "${CACHE_FORMAT_DEF}" | shasum -a 256 | head -c 64)"
CACHE_DIR="${CACHE_ROOT}/test-cache-${HASH}"

export AUTOBIB_RESPONSE_CACHE_PATH="${CACHE_DIR}/responses.dat"

if [[ "${LIBSQLITE3_SYS_USE_PKG_CONFIG:-0}" != "0" ]]; then
    FEATURE_ARGS=(--no-default-features)
else
    FEATURE_ARGS=()
fi

if [[ ! -f "${AUTOBIB_RESPONSE_CACHE_PATH}" ]]; then
    echo 2>&1 "Cache file not found. Generating cache file: ${AUTOBIB_RESPONSE_CACHE_PATH}"

    mkdir -p "${CACHE_DIR}"

    # Generate the cache file
    cargo run --locked "${FEATURE_ARGS[@]}" --features write_response_cache -- -vv source --retrieve-only --ignore-null "${REMOTES_FILE}"
else
    echo 2>&1 "Cache file found: ${AUTOBIB_RESPONSE_CACHE_PATH}"
fi

cargo test --locked --no-fail-fast "${FEATURE_ARGS[@]}" --features read_response_cache -- "$@"

cargo doc --no-deps --locked "${FEATURE_ARGS[@]}"
cargo clippy --locked "${FEATURE_ARGS[@]}"
cargo fmt --check
sort -C "${REMOTES_FILE}"
REPETITIONS="$(uniq -d < "${REMOTES_FILE}" | wc -w)"
test "${REPETITIONS}" -eq 0
shellcheck scripts/*.sh --enable=all
deno run --allow-read --allow-sys --no-config npm:markdownlint-cli2 -- '**/*.md' '!target/**/*.md'
