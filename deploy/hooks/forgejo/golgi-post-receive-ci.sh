#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# golgi-post-receive-ci.sh — sourDough validation + sovereign CI trigger on primal push
# Installed in each primal repo's hooks/post-receive.d/ on golgiBody.
#
# 1. Checks out HEAD to a temp dir and runs sourDough static validators
# 2. Triggers the build pipeline via membrane's mesh transport
#
# Static validators run inline (blocking); CI trigger runs in background.
# Validation failures are logged but do NOT block the push (advisory).
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
MEMBRANE_BIN="/opt/membrane/membrane"
SOURDOUGH_BIN="/opt/ecoPrimals/plasmidBin/primals/x86_64-unknown-linux-musl/sourdough"

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

# ── sourDough static validation (advisory — does not block push) ──
if [[ -x "$SOURDOUGH_BIN" ]]; then
    CHECKOUT_DIR=$(mktemp -d "/tmp/sourdough-ci-${PRIMAL_SLUG}-XXXXXX")
    if git --work-tree="$CHECKOUT_DIR" checkout HEAD -- . 2>/dev/null; then
        log "sourDough validate: $PRIMAL_SLUG commit=$COMMIT"

        VALIDATORS=("transport" "ribocipher" "platform-substrate" "neural-api")
        PASS=0
        FAIL=0
        for v in "${VALIDATORS[@]}"; do
            if "$SOURDOUGH_BIN" validate "$v" "$CHECKOUT_DIR" 2>/dev/null; then
                PASS=$((PASS + 1))
            else
                FAIL=$((FAIL + 1))
                log "WARN: sourDough validate $v FAILED for $PRIMAL_SLUG"
            fi
        done

        log "sourDough: $PRIMAL_SLUG $PASS pass, $FAIL fail (${#VALIDATORS[@]} validators)"
    else
        log "WARN: git checkout for sourDough validation failed"
    fi
    rm -rf "$CHECKOUT_DIR" 2>/dev/null
else
    log "SKIP: sourDough binary not found at $SOURDOUGH_BIN"
fi

# ── Sovereign CI trigger (background — mesh dispatch) ──
log "Triggering sovereign CI: $PRIMAL_SLUG commit=$COMMIT (mesh dispatch)"

ECOPRIMALS_ROOT=/opt/ecoPrimals "$MEMBRANE_BIN" \
    sovereign.ci.trigger --primal "$PRIMAL_SLUG" --commit "$COMMIT" \
    </dev/null >/dev/null 2>&1 &

log "CI trigger dispatched for $PRIMAL_SLUG (background)"
