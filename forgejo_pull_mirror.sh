#!/usr/bin/env bash
# forgejo_pull_mirror.sh — Manifest-driven Forgejo pull mirror management
#
# Converts Forgejo repos to pull mirrors from GitHub so Forgejo
# auto-syncs without any gate involvement. Uses ecosystem_manifest.toml
# as the single source of truth for which repos to mirror.
#
# Model: GitHub (outer membrane) is authoritative for public repos.
# Forgejo (inner membrane / periplasm) pulls from GitHub server-side.
# Private/inner-only repos are excluded (they're pushed directly).
#
# Usage:
#   ./forgejo_pull_mirror.sh --status          # Show mirror status for all repos
#   ./forgejo_pull_mirror.sh --dry-run         # Preview what would change
#   ./forgejo_pull_mirror.sh --migrate         # Delete + recreate as pull mirrors
#   ./forgejo_pull_mirror.sh --sync            # Trigger immediate sync on all mirrors
#
# Requires: FORGEJO_TOKEN env var or ~/.config/forgejo/token
#
# Coordination domain: waterFall (SYNC / autonomic)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

_find_manifest() {
    local candidates=(
        "${ECOPRIMALS_ROOT:-}/infra/wateringHole/ecosystem_manifest.toml"
        "$SCRIPT_DIR/../../infra/wateringHole/ecosystem_manifest.toml"
    )
    for c in "${candidates[@]}"; do
        [[ -f "$c" ]] && { echo "$c"; return; }
    done
    echo ""
}

MANIFEST="${ECOSYSTEM_MANIFEST:-$(_find_manifest)}"
if [[ -z "$MANIFEST" || ! -f "$MANIFEST" ]]; then
    echo "ERROR: ecosystem_manifest.toml not found"
    exit 1
fi

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
    echo >&2 "ERROR: FORGEJO_TOKEN required"
    return 1
}

FORGEJO_TOKEN="$(_resolve_token)" || exit 1
FORGEJO_API="${FORGEJO_API:-https://git.primals.eco/api/v1}"
AUTH="Authorization: token $FORGEJO_TOKEN"
MIRROR_INTERVAL="${MIRROR_INTERVAL:-8h0m0s}"

DRY_RUN=false
MIGRATE=false
SYNC=false
STATUS=false

for arg in "$@"; do
    case "$arg" in
        --dry-run)  DRY_RUN=true ;;
        --migrate)  MIGRATE=true ;;
        --sync)     SYNC=true ;;
        --status)   STATUS=true ;;
        --help|-h)
            cat <<'USAGE'
Usage: forgejo_pull_mirror.sh [--status|--dry-run|--migrate|--sync]

  --status    Show mirror status for all manifest repos
  --dry-run   Preview migration without making changes
  --migrate   Delete plain repos + recreate as pull mirrors from GitHub
  --sync      Trigger immediate sync on all existing pull mirrors

Excluded: inner-only repos, outer-only repos, repos without github_repo.
Interval: 8h (configurable via MIRROR_INTERVAL env var)

Token: Set FORGEJO_TOKEN or store at ~/.config/forgejo/token
USAGE
            exit 0 ;;
        *) echo "Unknown arg: $arg"; exit 1 ;;
    esac
done

if ! $STATUS && ! $DRY_RUN && ! $MIGRATE && ! $SYNC; then
    echo "ERROR: Specify --status, --dry-run, --migrate, or --sync"
    exit 1
fi

_py_read_manifest() {
    python3 -c "
import sys, json
try:
    import tomllib
except ImportError:
    import tomli as tomllib

with open('$MANIFEST', 'rb') as f:
    m = tomllib.load(f)

for key, repo in sorted(m.get('repos', {}).items()):
    fr = repo.get('forgejo_repo', '')
    gr = repo.get('github_repo', '')
    mem = repo.get('membrane', 'trailing-mirror')
    if fr and gr and mem not in ('inner-only', 'outer-only'):
        print(json.dumps({
            'key': key,
            'forgejo_repo': fr,
            'github_repo': gr,
            'membrane': mem,
        }))
"
}

migrated=0
failed=0
synced=0
skipped=0
already_mirror=0

echo "=== Forgejo Pull Mirror Manager ==="
echo "Manifest: $MANIFEST"
echo "API:      $FORGEJO_API"
echo "Interval: $MIRROR_INTERVAL"
echo ""

while IFS= read -r repo_json; do
    key=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['key'])")
    forgejo_repo=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['forgejo_repo'])")
    github_repo=$(echo "$repo_json" | python3 -c "import json,sys; print(json.load(sys.stdin)['github_repo'])")
    github_url="https://github.com/$github_repo.git"
    org="${forgejo_repo%%/*}"
    repo_name="${forgejo_repo##*/}"

    if $STATUS; then
        info=$(curl -sf --max-time 10 -H "$AUTH" "$FORGEJO_API/repos/$forgejo_repo" 2>/dev/null)
        if [[ -z "$info" ]]; then
            printf "  %-25s %-35s NOT ON FORGEJO\n" "$key" "$forgejo_repo"
            continue
        fi
        is_mirror=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
        if [[ "$is_mirror" == "True" ]]; then
            interval=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_interval','?'))" 2>/dev/null)
            updated=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_updated','?')[:19])" 2>/dev/null)
            printf "  %-25s %-35s ✓ MIRROR  interval=%s  last=%s\n" "$key" "$forgejo_repo" "$interval" "$updated"
        else
            printf "  %-25s %-35s PLAIN REPO (needs migration)\n" "$key" "$forgejo_repo"
        fi
        continue
    fi

    if $SYNC; then
        code=$(curl -sf --max-time 10 -X POST -H "$AUTH" "$FORGEJO_API/repos/$forgejo_repo/mirror-sync" -w "%{http_code}" -o /dev/null 2>/dev/null)
        if [[ "$code" == "200" ]]; then
            printf "  %-25s SYNC TRIGGERED\n" "$key"
            ((synced++))
        else
            printf "  %-25s SYNC SKIP (not a mirror or error: HTTP %s)\n" "$key" "$code"
            ((skipped++))
        fi
        continue
    fi

    exists=$(curl -sf --max-time 10 -o /dev/null -w '%{http_code}' -H "$AUTH" "$FORGEJO_API/repos/$forgejo_repo" 2>/dev/null || echo "000")

    if [[ "$exists" == "200" ]]; then
        is_mirror=$(curl -sf --max-time 10 -H "$AUTH" "$FORGEJO_API/repos/$forgejo_repo" 2>/dev/null | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
        if [[ "$is_mirror" == "True" ]]; then
            printf "  %-25s ALREADY MIRROR (skip)\n" "$key"
            ((already_mirror++))
            continue
        fi
    fi

    if $DRY_RUN; then
        if [[ "$exists" == "200" ]]; then
            printf "  %-25s WOULD DELETE + RECREATE as mirror ← %s\n" "$key" "$github_url"
        else
            printf "  %-25s WOULD CREATE mirror ← %s\n" "$key" "$github_url"
        fi
        continue
    fi

    # --migrate: delete existing plain repo and recreate as pull mirror
    if [[ "$exists" == "200" ]]; then
        del_code=$(curl -sf --max-time 10 -X DELETE -H "$AUTH" "$FORGEJO_API/repos/$forgejo_repo" -w "%{http_code}" -o /dev/null 2>/dev/null)
        if [[ "$del_code" != "204" ]]; then
            printf "  %-25s DELETE FAILED (HTTP %s)\n" "$key" "$del_code"
            ((failed++))
            continue
        fi
        sleep 0.5
    fi

    migrate_result=$(curl -sf --max-time 30 -X POST -H "$AUTH" -H "Content-Type: application/json" \
        "$FORGEJO_API/repos/migrate" \
        -d "{
            \"clone_addr\": \"$github_url\",
            \"repo_name\": \"$repo_name\",
            \"repo_owner\": \"$org\",
            \"mirror\": true,
            \"mirror_interval\": \"$MIRROR_INTERVAL\",
            \"private\": false,
            \"service\": \"github\"
        }" 2>&1)

    is_mirror=$(echo "$migrate_result" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
    if [[ "$is_mirror" == "True" ]]; then
        printf "  %-25s MIGRATED → mirror (interval=%s)\n" "$key" "$MIRROR_INTERVAL"
        ((migrated++))
    else
        msg=$(echo "$migrate_result" | python3 -c "import json,sys; print(json.load(sys.stdin).get('message','unknown'))" 2>/dev/null || echo "unknown")
        printf "  %-25s MIGRATE FAILED: %s\n" "$key" "$msg"
        ((failed++))
    fi
    sleep 0.3

done < <(_py_read_manifest)

echo ""
if $STATUS; then
    echo "Status check complete."
elif $SYNC; then
    echo "Summary: $synced triggered, $skipped skipped"
else
    echo "Summary: $migrated migrated, $already_mirror already mirrors, $skipped skipped, $failed failed"
fi
