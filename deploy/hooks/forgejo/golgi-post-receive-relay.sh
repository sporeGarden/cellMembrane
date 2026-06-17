#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# golgi-post-receive-relay.sh — Rust-native K-Derm relay hook
# Installed in each repo's hooks/post-receive.d/ on golgiBody-inner.
# Triggers membrane relay.run (Rust binary) instead of bash SSH chain.
#
# Forgejo cd's into the bare repo dir and sets GIT_DIR=. before running
# hooks, so we resolve via pwd to get the actual repo path.

set -uo pipefail

LOG_TAG="golgi-relay-rust"
log() { logger -t "$LOG_TAG" "$@" 2>/dev/null || echo "[$LOG_TAG] $*"; }

REPO_BARE="$(cd "${GIT_DIR:-.}" 2>/dev/null && pwd)"
REPO_NAME=$(basename "$REPO_BARE" .git)

MANIFEST="/opt/ecoPrimals/infra/wateringHole/ecosystem_manifest.toml"
LOCAL_PATH=""
if [[ -f "$MANIFEST" ]]; then
    LOCAL_PATH=$(grep -iB10 "forgejo_repo.*${REPO_NAME}" "$MANIFEST" 2>/dev/null \
        | grep "local_path" | tail -1 | sed 's/.*= *"//' | sed 's/".*//')
fi

if [[ -z "${LOCAL_PATH}" ]]; then
    log "WARN: Could not resolve local_path for $REPO_NAME (bare=$REPO_BARE), skipping relay"
    exit 0
fi

log "Post-receive relay (Rust): $REPO_NAME -> $LOCAL_PATH"

ECOPRIMALS_ROOT=/opt/ecoPrimals /usr/local/bin/membrane relay.run "$LOCAL_PATH" \
    </dev/null >/dev/null 2>&1 &

log "Relay triggered for $LOCAL_PATH (background)"
