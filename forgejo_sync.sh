#!/usr/bin/env bash
# forgejo_sync.sh — Manifest-driven Forgejo membrane sync
#
# Reads ecosystem_manifest.toml and works with Forgejo's mirror topology:
# - Pull mirrors (36 repos): triggers API sync so Forgejo re-pulls from GitHub
# - Inner-only repos (cellMembrane): pushes directly to Forgejo
# - Outer-only repos (sporePrint): skipped
#
# Usage:
#   ./forgejo_sync.sh                  # Sync all repos (trigger mirrors + push inner-only)
#   ./forgejo_sync.sh --status         # Show sync status without action
#   ./forgejo_sync.sh --check          # Quick parity check (no push/trigger)
#   ./forgejo_sync.sh --force          # Force-push diverged inner-only repos
#   ./forgejo_sync.sh --mirrors-only   # Only trigger Forgejo mirror sync
#   ./forgejo_sync.sh --push-only      # Only push inner-only repos
#
# Token resolution:
#   1. FORGEJO_TOKEN env var
#   2. ~/.config/forgejo/token file
#
# Manifest: ecosystem_manifest.toml (auto-discovered from script location)
# Coordination domain: waterFall (SYNC / autonomic)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

_find_manifest() {
    local candidates=(
        "${ECOPRIMALS_ROOT:-}/infra/wateringHole/ecosystem_manifest.toml"
        "$SCRIPT_DIR/../../infra/wateringHole/ecosystem_manifest.toml"
        "$SCRIPT_DIR/../wateringHole/ecosystem_manifest.toml"
    )
    for c in "${candidates[@]}"; do
        [[ -f "$c" ]] && { echo "$c"; return; }
    done
    echo ""
}

MANIFEST="${ECOSYSTEM_MANIFEST:-$(_find_manifest)}"
if [[ -z "$MANIFEST" || ! -f "$MANIFEST" ]]; then
    echo "ERROR: ecosystem_manifest.toml not found"
    echo "Hint: set ECOPRIMALS_ROOT or ECOSYSTEM_MANIFEST"
    exit 1
fi

_resolve_root() {
    local mdir
    mdir="$(cd "$(dirname "$MANIFEST")/../.." && pwd)"
    echo "$mdir"
}

ECOPRIMALS_ROOT="${ECOPRIMALS_ROOT:-$(_resolve_root)}"

_resolve_token() {
    if [[ -n "${FORGEJO_TOKEN:-}" ]]; then
        echo "$FORGEJO_TOKEN"
        return
    fi
    local token_file="$HOME/.config/forgejo/token"
    if [[ -f "$token_file" ]]; then
        cat "$token_file"
        return
    fi
    echo ""
}

FORGEJO_TOKEN="$(_resolve_token)"
FORGEJO_API="${FORGEJO_API:-https://git.primals.eco/api/v1}"
LOGFILE="${LOGFILE:-${XDG_STATE_HOME:-$HOME/.local/state}/forgejo/sync.log}"

FORCE=false
STATUS_ONLY=false
CHECK_ONLY=false
MIRRORS_ONLY=false
PUSH_ONLY=false

for arg in "$@"; do
    case "$arg" in
        --force)          FORCE=true ;;
        --status)         STATUS_ONLY=true ;;
        --check)          CHECK_ONLY=true ;;
        --mirrors-only)   MIRRORS_ONLY=true ;;
        --push-only)      PUSH_ONLY=true ;;
        --help|-h)
            cat <<'USAGE'
Usage: forgejo_sync.sh [OPTIONS]

Options:
  --status          Show mirror/sync status for all repos
  --check           Quick parity check (no push, no trigger)
  --force           Use --force-with-lease for diverged inner-only repos
  --mirrors-only    Only trigger Forgejo mirror sync (API)
  --push-only       Only push inner-only repos to Forgejo
  -h, --help        Show this help

Sync model:
  Pull mirrors: Forgejo auto-syncs from GitHub every 8h. This script
  triggers immediate sync via API when you want faster convergence.
  Inner-only repos (e.g. cellMembrane): pushed directly to Forgejo.

Token: Set FORGEJO_TOKEN env var, or store at ~/.config/forgejo/token
Coordination domain: waterFall (SYNC / autonomic)
USAGE
            exit 0 ;;
        *) echo "Unknown option: $arg"; exit 1 ;;
    esac
done

_py_read_manifest() {
    python3 -c "
import sys, json
try:
    import tomllib
except ImportError:
    import tomli as tomllib

with open('$MANIFEST', 'rb') as f:
    m = tomllib.load(f)

cmd = sys.argv[1]

if cmd == 'all_repos':
    sync = m.get('sync', {})
    forgejo_ssh = sync.get('forgejo_ssh', 'ssh://git@git.primals.eco:2222')
    for key, repo in sorted(m.get('repos', {}).items()):
        lp = repo.get('local_path', '')
        fr = repo.get('forgejo_repo', '')
        gr = repo.get('github_repo', '')
        mem = repo.get('membrane', 'trailing-mirror')
        if lp and fr:
            print(json.dumps({
                'key': key,
                'local_path': lp,
                'forgejo_repo': fr,
                'github_repo': gr,
                'membrane': mem,
                'forgejo_ssh': f'{forgejo_ssh}/{fr}.git',
            }))
" "$@"
}

mkdir -p "$(dirname "$LOGFILE")"

echo "=== Forgejo Membrane Sync ==="
echo "Manifest: $MANIFEST"
echo "Root:     $ECOPRIMALS_ROOT"
echo "API:      $FORGEJO_API"
echo "Token:    $([ -n "$FORGEJO_TOKEN" ] && echo 'configured' || echo 'MISSING')"
echo ""

mirror_triggered=0
push_ok=0
push_fail=0
already_sync=0
skipped=0

while IFS= read -r repo_json; do
    key=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['key'])")
    local_path=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['local_path'])")
    forgejo_repo=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['forgejo_repo'])")
    membrane=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['membrane'])")
    forgejo_ssh=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['forgejo_ssh'])")

    full="$ECOPRIMALS_ROOT/$local_path"

    if [[ "$membrane" == "outer-only" ]]; then
        $STATUS_ONLY && printf "  %-30s OUTER-ONLY (skip)\n" "$key"
        ((skipped++))
        continue
    fi

    # ── Status mode ──────────────────────────────────────────────
    if $STATUS_ONLY && [[ -n "$FORGEJO_TOKEN" ]]; then
        info=$(curl -sf --max-time 10 -H "Authorization: token $FORGEJO_TOKEN" "$FORGEJO_API/repos/$forgejo_repo" 2>/dev/null)
        if [[ -z "$info" ]]; then
            printf "  %-30s NOT ON FORGEJO\n" "$key"
        else
            is_mirror=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
            if [[ "$is_mirror" == "True" ]]; then
                interval=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_interval','?'))" 2>/dev/null)
                updated=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_updated','?')[:19])" 2>/dev/null)
                printf "  %-30s ✓ MIRROR  interval=%s  last=%s\n" "$key" "$interval" "$updated"
            else
                printf "  %-30s REPO (not mirror, membrane=%s)\n" "$key" "$membrane"
            fi
        fi
        continue
    fi

    # ── Check mode ───────────────────────────────────────────────
    if $CHECK_ONLY; then
        if [[ ! -d "$full/.git" ]]; then
            printf "  %-30s NOT CLONED\n" "$key"
            continue
        fi
        branch=$(git -C "$full" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "main")
        local_ref=$(git -C "$full" rev-parse HEAD 2>/dev/null || echo "none")

        if git -C "$full" remote get-url forgejo >/dev/null 2>&1; then
            git -C "$full" fetch forgejo "$branch" --quiet 2>/dev/null || true
            fg_ref=$(git -C "$full" rev-parse "forgejo/$branch" 2>/dev/null || echo "none")
        else
            fg_ref="none"
        fi

        if [[ "$local_ref" == "$fg_ref" ]]; then
            printf "  %-30s PARITY\n" "$key"
            ((already_sync++))
        elif [[ "$fg_ref" == "none" ]]; then
            printf "  %-30s NO FORGEJO REF\n" "$key"
        else
            ahead=$(git -C "$full" rev-list --count "forgejo/$branch".."$branch" 2>/dev/null || echo "?")
            behind=$(git -C "$full" rev-list --count "$branch".."forgejo/$branch" 2>/dev/null || echo "?")
            printf "  %-30s DRIFT (ahead=%s behind=%s)\n" "$key" "$ahead" "$behind"
        fi
        continue
    fi

    # ── Sync mode ────────────────────────────────────────────────
    # For mirror repos: trigger API sync (Forgejo pulls from GitHub)
    # For inner-only repos: push directly to Forgejo

    if [[ "$membrane" == "inner-only" ]]; then
        # Inner-only repo: push directly to Forgejo
        if $MIRRORS_ONLY; then
            continue
        fi

        if [[ ! -d "$full/.git" ]]; then
            printf "  %-30s NOT CLONED (skip)\n" "$key"
            ((skipped++))
            continue
        fi

        branch=$(git -C "$full" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "main")

        if ! git -C "$full" remote get-url forgejo >/dev/null 2>&1; then
            git -C "$full" remote add forgejo "$forgejo_ssh" 2>/dev/null || true
        fi

        git -C "$full" fetch forgejo "$branch" --quiet 2>/dev/null || true
        fg_ref=$(git -C "$full" rev-parse "forgejo/$branch" 2>/dev/null || echo "none")
        local_ref=$(git -C "$full" rev-parse "$branch" 2>/dev/null)

        if [[ "$local_ref" == "$fg_ref" ]]; then
            printf "  %-30s UP TO DATE (inner-only)\n" "$key"
            ((already_sync++))
            continue
        fi

        push_flags=""
        $FORCE && push_flags="--force-with-lease"

        if git -C "$full" push $push_flags forgejo "$branch" >/dev/null 2>&1; then
            ahead=$(git -C "$full" rev-list --count "forgejo/$branch".."$branch" 2>/dev/null || echo "?")
            printf "  %-30s PUSHED (+%s) (inner-only)\n" "$key" "$ahead"
            ((push_ok++))
        else
            printf "  %-30s PUSH FAILED (inner-only)\n" "$key"
            ((push_fail++))
        fi
    else
        # Mirror repo: trigger Forgejo to re-pull from GitHub
        if $PUSH_ONLY; then
            continue
        fi

        if [[ -n "$FORGEJO_TOKEN" ]]; then
            code=$(curl -sf --max-time 10 -X POST -H "Authorization: token $FORGEJO_TOKEN" "$FORGEJO_API/repos/$forgejo_repo/mirror-sync" -w "%{http_code}" -o /dev/null 2>/dev/null)
            if [[ "$code" == "200" ]]; then
                printf "  %-30s SYNC TRIGGERED (mirror)\n" "$key"
                ((mirror_triggered++))
            else
                printf "  %-30s SYNC FAILED HTTP %s (mirror)\n" "$key" "$code"
                ((push_fail++))
            fi
        else
            printf "  %-30s NO TOKEN (mirror sync skipped)\n" "$key"
            ((skipped++))
        fi
    fi
done < <(_py_read_manifest all_repos)

echo ""
echo "=== Summary ==="
echo "Mirrors triggered: $mirror_triggered"
[[ $push_ok -gt 0 ]] && echo "Pushed (inner-only): $push_ok"
[[ $already_sync -gt 0 ]] && echo "Already synced: $already_sync"
[[ $push_fail -gt 0 ]] && echo "Failed: $push_fail"
[[ $skipped -gt 0 ]] && echo "Skipped: $skipped"
echo "$(date '+%Y-%m-%d %H:%M:%S') mirrors=$mirror_triggered pushed=$push_ok failed=$push_fail" >> "$LOGFILE"
