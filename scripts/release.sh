#!/usr/bin/env bash
set -euo pipefail

die() {
	echo "$@" >&2
	exit 1
}

VALID_ARGS_MSG="[major, minor, patch, rc, beta, alpha]"
if [[ "$#" -eq 0 ]]; then
    die "No arguments provided: expected one of: ${VALID_ARGS_MSG}"
fi

if [[ "$#" -ge 2 ]]; then
    die "Too many arguments: expected exactly one argument from: ${VALID_ARGS_MSG}"
fi

PROJECT_ROOT="$(cargo locate-project --message-format plain)"
cd "$(dirname "${PROJECT_ROOT}")"

GIT_STATUS="$(git status --porcelain)"
if [[ -n "${GIT_STATUS}" ]]; then 
    die "Git repository is dirty; commit or stash changes before continuing."
fi

CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "${CURRENT_BRANCH}" != "main" ]]; then
    die "New releases should be cut from 'main'; current branch is '${CURRENT_BRANCH}'"
fi

cargo set-version --bump "$1"
VERSION="$(yq '.package.version' Cargo.toml)"
VERSION_TAG="v${VERSION}"

DATE_FORMATTED="$(date +"%Y-%m-%d")"
NEW_TITLE="# Version ${VERSION} (${DATE_FORMATTED})"

echo "Renaming 'docs/changelog/next.md' to 'docs/changelog/${VERSION_TAG}.md'" and updating title
sed -z 's/^[[:space:]]*# Unreleased/'"${NEW_TITLE}"'/g' docs/changelog/next.md > "docs/changelog/${VERSION_TAG}.md"
echo "# Unreleased" > docs/changelog/next.md

NEW_BRANCH="create-release-${VERSION_TAG}"
git branch "${NEW_BRANCH}"
git switch "${NEW_BRANCH}"
git add "docs/changelog/${VERSION_TAG}.md" "Cargo.toml" "Cargo.lock" "docs/changelog/next.md"
git commit -m "Release version ${VERSION}"
