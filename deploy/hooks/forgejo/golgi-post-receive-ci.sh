#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# golgi-post-receive-ci.sh — Sovereign CI trigger on primal push
# Installed in each primal repo's hooks/post-receive.d/ on golgiBody.
# Triggers sporeGate build pipeline via WireGuard SSH when a primal is pushed.
#
# Install:
#   for repo in /opt/forgejo/data/gitea-repositories/ecoPrimals/*.git; do
#       name=$(basename "$repo" .git)
#       mkdir -p "$repo/hooks/post-receive.d"
#       cp golgi-post-receive-ci.sh "$repo/hooks/post-receive.d/30-sovereign-ci"
#       chmod +x "$repo/hooks/post-receive.d/30-sovereign-ci"
#   done

set -uo pipefail

LOG_TAG="golgi-ci-trigger"
log() { logger -t "$LOG_TAG" "$@" 2>/dev/null || echo "[$LOG_TAG] $*"; }

REPO_BARE="$(cd "${GIT_DIR:-.}" 2>/dev/null && pwd)"
REPO_NAME=$(basename "$REPO_BARE" .git)

MANIFEST="/opt/ecoPrimals/infra/wateringHole/ecosystem_manifest.toml"
SPOREGATE_MESH_IP="10.13.37.2"
MEMBRANE_BIN="/usr/local/bin/membrane"

if [[ ! -f "$MANIFEST" ]]; then
    log "SKIP: ecosystem manifest not found"
    exit 0
fi

IS_PRIMAL=$(grep -A5 "^\[repos\.$REPO_NAME\]" "$MANIFEST" 2>/dev/null \
    | grep -c 'category.*=.*"primals"' || true)

if [[ "$IS_PRIMAL" -eq 0 ]]; then
    log "SKIP: $REPO_NAME is not a primal repo"
    exit 0
fi

COMMIT=$(cd "$REPO_BARE" && git rev-parse HEAD 2>/dev/null | head -c 12)
if [[ -z "$COMMIT" ]]; then
    log "WARN: could not resolve HEAD for $REPO_NAME"
    exit 0
fi

PRIMAL_SLUG=$(echo "$REPO_NAME" | tr '[:upper:]' '[:lower:]')

log "Triggering sovereign CI: $PRIMAL_SLUG commit=$COMMIT on sporeGate ($SPOREGATE_MESH_IP)"

ssh -o StrictHostKeyChecking=accept-new \
    -o ConnectTimeout=5 \
    -o BatchMode=yes \
    "root@$SPOREGATE_MESH_IP" \
    "$MEMBRANE_BIN sovereign.ci.trigger --primal $PRIMAL_SLUG --commit $COMMIT" \
    </dev/null >/dev/null 2>&1 &

log "CI trigger dispatched for $PRIMAL_SLUG (background)"
