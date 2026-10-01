#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# golgi-post-receive-github-mirror.sh — Push bare repo to GitHub mirror
# Installed as hooks/post-receive.d/70-github-mirror on golgiBody.
#
# One-way push from Forgejo bare repo → GitHub public mirror.
# Only pushes if the repo has a "github" remote configured.
# Uses --force-with-lease for safe divergence handling.
#
# Prerequisites:
#   - GitHub repo exists (e.g. ecoPrimals/sporePrint)
#   - Bare repo has `github` remote: git remote add github git@github.com:ecoPrimals/<repo>.git
#   - SSH key authorized for GitHub push

set -uo pipefail

LOG_TAG="github-mirror"
log() { logger -t "$LOG_TAG" "$@" 2>/dev/null || echo "[$LOG_TAG] $*"; }

REPO_BARE="$(cd "${GIT_DIR:-.}" 2>/dev/null && pwd)"
REPO_NAME=$(basename "$REPO_BARE" .git)

if ! git -C "$REPO_BARE" remote get-url github >/dev/null 2>&1; then
    log "No github remote for $REPO_NAME — skipping mirror"
    exit 0
fi

BRANCH=$(git -C "$REPO_BARE" symbolic-ref --short HEAD 2>/dev/null || echo main)

log "Mirroring $REPO_NAME ($BRANCH) → GitHub"

git -C "$REPO_BARE" push --force-with-lease github "$BRANCH" --quiet 2>&1 \
    | while IFS= read -r line; do log "$line"; done &

log "GitHub mirror push triggered for $REPO_NAME (background)"
