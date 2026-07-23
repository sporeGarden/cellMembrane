#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# golgi-post-receive-ci.sh — Sovereign CI dispatch on push
# Installed in each primal repo's hooks/post-receive.d/ on golgiBody.
#
# When a primal repo receives a push, SSH to sporeGate (build authority)
# and trigger `membrane sovereign.ci.trigger` to rebuild the binary,
# sandbox-validate, and sync to depot.
#
# Non-primal repos (infra, gardens, springs) are silently skipped.
#
# Architecture:
#   Forgejo push → this hook → SSH sporeGate → sovereign.ci.trigger
#     → harvest (cargo build musl) → sandbox → refresh → depot_sync
#     → golgiBody depot updated → gates auto-fetch
#
# Install (on golgiBody, for each primal repo):
#   cp golgi-post-receive-ci.sh <repo>.git/hooks/post-receive.d/30-sovereign-ci
#   chmod +x <repo>.git/hooks/post-receive.d/30-sovereign-ci

set -uo pipefail

LOG_TAG="golgi-sovereign-ci"
log() { logger -t "$LOG_TAG" "$@" 2>/dev/null || echo "[$LOG_TAG] $*"; }

REPO_BARE="$(cd "${GIT_DIR:-.}" 2>/dev/null && pwd)"
REPO_NAME=$(basename "$REPO_BARE" .git)

MANIFEST="/opt/ecoPrimals/infra/wateringHole/ecosystem_manifest.toml"

# Resolve the primal name and category from the ecosystem manifest.
# Only dispatch CI for repos in the "primals" category.
CATEGORY=""
PRIMAL_NAME=""
if [[ -f "$MANIFEST" ]]; then
    SECTION=$(grep -iB20 "forgejo_repo.*${REPO_NAME}" "$MANIFEST" 2>/dev/null)
    CATEGORY=$(echo "$SECTION" | grep "category" | tail -1 | sed 's/.*= *"//' | sed 's/".*//')
    PRIMAL_NAME=$(echo "$SECTION" | grep -oP '^\[repos\.\K[^]]+' | tail -1)
fi

if [[ "$CATEGORY" != "primals" ]]; then
    exit 0
fi

if [[ -z "$PRIMAL_NAME" ]]; then
    log "WARN: Could not resolve primal name for $REPO_NAME, skipping CI"
    exit 0
fi

# Read the pushed commit SHA from stdin (Forgejo provides oldrev newrev refname).
COMMIT=""
while read -r _oldrev newrev refname; do
    if [[ "$refname" == "refs/heads/main" ]]; then
        COMMIT="${newrev:0:12}"
        break
    fi
done

if [[ -z "$COMMIT" ]]; then
    log "Push to $REPO_NAME was not to main branch, skipping CI"
    exit 0
fi

# sporeGate is the build authority — dispatch over WireGuard mesh.
BUILDER_HOST="10.13.37.2"
BUILDER_USER="${SOVEREIGN_CI_USER:-eastgate}"

log "Sovereign CI: $PRIMAL_NAME @ $COMMIT → $BUILDER_HOST"

ssh -o ConnectTimeout=5 \
    -o StrictHostKeyChecking=accept-new \
    -o BatchMode=yes \
    "${BUILDER_USER}@${BUILDER_HOST}" \
    "ECOPRIMALS_ROOT=/opt/ecoPrimals /usr/local/bin/membrane sovereign.ci.trigger --primal ${PRIMAL_NAME} --commit ${COMMIT}" \
    </dev/null >/dev/null 2>&1 &

log "Sovereign CI dispatched for $PRIMAL_NAME (background, pid=$!)"
