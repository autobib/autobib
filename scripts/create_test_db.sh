#!/usr/bin/env bash
set -euo pipefail

CACHE_HOME="${XDG_CACHE_HOME:=${HOME}/.cache/}"
CACHE_ROOT="${AUTOBIB_RESPONSE_CACHE_DIR:=${CACHE_HOME}/autobib}"

mkdir -p "${CACHE_ROOT}"
HASH="$(git rev-parse HEAD)"
export AUTOBIB_DATABASE_PATH="${CACHE_ROOT}/temp-database-${HASH}.db"

rm -f "${AUTOBIB_DATABASE_PATH}"

echo "Using database file:"
echo "${AUTOBIB_DATABASE_PATH}"
echo

cargo build

autobib="./target/debug/autobib --quiet"

${autobib} local first \
    --with-entry-type "book" \
    --with-field "author = {1}" \
    --with-field "title = {2}"

${autobib} local second \
    --with-entry-type "article" \
    --with-field "author = {A}"

${autobib} edit local:first \
    --delete-field "author"

${autobib} hist undo local:first

${autobib} edit local:first \
    --set-field "title = {3}"

${autobib} hist undo local:first

${autobib} hist redo local:first 0

${autobib} edit local:first \
    --set-field "title = {4}"

${autobib} edit local:first \
    --set-field "title = {5}"

${autobib} replace local:first --with local:second

${autobib} hist revive local:first \
    --with-field "title = {6}"

${autobib} edit local:second \
    --update-entry-type manuscript

${autobib} hist undo local:first --delete

${autobib} hist undo local:first

${autobib} delete local:first

${autobib} hist revive local:first \
    --with-field "author = {B}" \
    --with-entry-type book

${autobib} hist void local:first

${autobib} hist revive local:first \
    --with-field "author = {C}" \
    --with-entry-type article

${autobib} log local:first --tree --all
