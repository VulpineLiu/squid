#!/usr/bin/env bash
# Rebuild the public GitHub mirror without release installers.
#
# GitHub is a source mirror, not an update channel. Its history is rewritten to
# drop `release/`, `*.dmg` and `*.exe`, so its commit hashes differ from origin.
# Never push the working repository's `main` directly to GitHub; run this script
# after pushing origin/main instead.
#
#   scripts/sync-github.sh
#
# Override the destination with SQUID_GITHUB_URL, or the branch with
# SQUID_GITHUB_BRANCH.
set -euo pipefail
cd "$(dirname "$0")/.."

BRANCH="${SQUID_GITHUB_BRANCH:-main}"
URL="${SQUID_GITHUB_URL:-https://github.com/VulpineLiu/squid.git}"
WORK="$(mktemp -d /tmp/squid-github-mirror.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

git clone --no-local --single-branch --branch "$BRANCH" . "$WORK/repo"
cd "$WORK/repo"

FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch --force \
  --index-filter 'git rm -r --cached --ignore-unmatch release "*.dmg" "*.exe"' \
  --prune-empty -- --all

git for-each-ref --format='%(refname)' refs/original/ | while read -r ref; do
  git update-ref -d "$ref"
done
git reflog expire --expire=now --all
git gc --prune=now --aggressive >/dev/null

git remote add github "$URL"
git push --force github "$BRANCH:$BRANCH"
