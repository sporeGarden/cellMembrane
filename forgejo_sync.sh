#!/usr/bin/env bash
# forgejo_sync.sh — Sync non-mirror repos to Forgejo inner membrane
#
# Model: GitHub (external membrane) is authoritative. Forgejo (inner membrane)
# trails behind. 25/31 repos are native Forgejo pull mirrors that auto-sync
# every 8h. The remaining 6 repos (3 private, 3 large/public) couldn't become
# native mirrors, so this script fetches from GitHub and pushes to Forgejo.
#
# When covalent gates host Forgejo, we invert: Forgejo becomes primary.
#
# Usage:
#   ./forgejo_sync.sh              # Sync the 6 non-mirror repos
#   ./forgejo_sync.sh --status     # Show sync status without pushing
#   ./forgejo_sync.sh --all        # Also trigger mirror-sync on native mirrors
#   ./forgejo_sync.sh --force      # Force-push diverged repos
#
# Designed for cron/systemd timer on the Forgejo host machine (ironGate).
# Does NOT require the dev machine to be the one that pushed to GitHub.

set -uo pipefail

ECOPRIMALS_ROOT="${ECOPRIMALS_ROOT:-$HOME/Development/ecoPrimals}"
FORGEJO_URL="${FORGEJO_URL:-http://127.0.0.1:3000}"
FORGEJO_TOKEN="${FORGEJO_TOKEN:-}"
LOGFILE="${LOGFILE:-/tmp/forgejo_sync.log}"
FORCE=false
STATUS_ONLY=false
SYNC_ALL=false

for arg in "$@"; do
  case "$arg" in
    --force)    FORCE=true ;;
    --status)   STATUS_ONLY=true ;;
    --all)      SYNC_ALL=true ;;
    --help|-h)
      echo "Usage: $0 [--status] [--force] [--all]"
      echo "  --status   Show sync status without pushing"
      echo "  --force    Use --force-with-lease for diverged repos"
      echo "  --all      Also trigger native mirror sync via API"
      exit 0 ;;
    *) echo "Unknown arg: $arg" >&2; exit 1 ;;
  esac
done

if ! curl -sf "$FORGEJO_URL/api/v1/version" --max-time 5 >/dev/null 2>&1; then
  echo "$(date '+%H:%M:%S') Forgejo not reachable — skipping" | tee -a "$LOGFILE"
  exit 0
fi

sync_ok=0
sync_fail=0
already_sync=0

# Non-mirror repos: fetch from GitHub, push to Forgejo
declare -A NON_MIRROR_REPOS
NON_MIRROR_REPOS["primals/bearDog"]="ecoPrimals/bearDog"
NON_MIRROR_REPOS["primals/skunkBat"]="ecoPrimals/skunkBat"
NON_MIRROR_REPOS["infra/whitePaper"]="ecoPrimals/whitePaper"
NON_MIRROR_REPOS["springs/neuralSpring"]="syntheticChemistry/neuralSpring"
NON_MIRROR_REPOS["springs/primalSpring"]="syntheticChemistry/primalSpring"
NON_MIRROR_REPOS["springs/wetSpring"]="syntheticChemistry/wetSpring"

echo "Forgejo sync — $(date '+%Y-%m-%d %H:%M:%S')"
echo ""
echo "=== Non-mirror repos (fetch origin → push forgejo) ==="

for local_dir in $(printf '%s\n' "${!NON_MIRROR_REPOS[@]}" | sort); do
  forgejo_path="${NON_MIRROR_REPOS[$local_dir]}"
  full="$ECOPRIMALS_ROOT/$local_dir"

  if [ ! -d "$full/.git" ]; then
    printf "  %-30s MISSING\n" "$local_dir"
    continue
  fi

  branch=$(git -C "$full" symbolic-ref --short HEAD 2>/dev/null || echo "main")

  # Fetch latest from GitHub (origin)
  if ! git -C "$full" fetch origin --quiet 2>/dev/null; then
    printf "  %-30s FETCH FAILED\n" "$local_dir"
    ((sync_fail++))
    continue
  fi

  # Fast-forward local to origin if behind
  local_ref=$(git -C "$full" rev-parse "$branch" 2>/dev/null)
  origin_ref=$(git -C "$full" rev-parse "origin/$branch" 2>/dev/null || echo "none")
  if [ "$local_ref" != "$origin_ref" ] && [ "$origin_ref" != "none" ]; then
    git -C "$full" merge --ff-only "origin/$branch" --quiet 2>/dev/null || true
  fi

  # Check if Forgejo is already up to date
  git -C "$full" fetch forgejo --quiet 2>/dev/null
  fg_ref=$(git -C "$full" rev-parse "forgejo/$branch" 2>/dev/null || echo "none")
  local_ref=$(git -C "$full" rev-parse "$branch" 2>/dev/null)

  if [ "$local_ref" = "$fg_ref" ]; then
    printf "  %-30s UP TO DATE\n" "$local_dir"
    ((already_sync++))
    continue
  fi

  ahead=$(git -C "$full" rev-list --count "forgejo/$branch".."$branch" 2>/dev/null || echo "?")

  if $STATUS_ONLY; then
    printf "  %-30s ahead +%s\n" "$local_dir" "$ahead"
    continue
  fi

  push_flags=""
  if $FORCE; then push_flags="--force-with-lease"; fi

  if git -C "$full" push $push_flags forgejo "$branch" >/dev/null 2>&1; then
    printf "  %-30s PUSHED (+%s)\n" "$local_dir" "$ahead"
    ((sync_ok++))
  else
    printf "  %-30s PUSH FAILED (diverged? use --force)\n" "$local_dir"
    ((sync_fail++))
  fi
done

# Optionally trigger sync on native pull mirrors
if $SYNC_ALL && [ -n "$FORGEJO_TOKEN" ]; then
  echo ""
  echo "=== Native pull mirrors (trigger sync via API) ==="
  AUTH="Authorization: token $FORGEJO_TOKEN"
  mirror_synced=0
  for path in ecoPrimals/{barraCuda,bingoCube,biomeOS,coralReef,loamSpine,nestGate,petalTongue,plasmidBin,rhizoCrypt,songBird,sourDough,sporePrint,squirrel,sweetGrass,toadStool,wateringHole} \
              sporeGarden/{esotericWebb,lithoSpore,projectFOUNDATION,projectNUCLEUS} \
              syntheticChemistry/{airSpring,groundSpring,healthSpring,hotSpring,ludoSpring}; do
    code=$(curl -sf -X POST -H "$AUTH" "$FORGEJO_URL/api/v1/repos/$path/mirror-sync" -w "%{http_code}" -o /dev/null 2>/dev/null)
    if [ "$code" = "200" ]; then
      ((mirror_synced++))
    else
      printf "  %-40s SYNC FAILED (HTTP %s)\n" "$path" "$code"
    fi
  done
  printf "  Triggered sync on %d native mirrors\n" "$mirror_synced"
fi

echo ""
echo "Summary: $sync_ok pushed, $already_sync up-to-date, $sync_fail failed"
echo "$(date '+%Y-%m-%d %H:%M:%S') | pushed=$sync_ok synced=$already_sync failed=$sync_fail" >> "$LOGFILE"
