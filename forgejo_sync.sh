#!/usr/bin/env bash
# forgejo_sync.sh — Async Forgejo mirror sync for ecoPrimals inner membrane
#
# Usage:
#   ./forgejo_sync.sh              # Sync all repos
#   ./forgejo_sync.sh --status     # Show sync status only
#   ./forgejo_sync.sh <repo-path>  # Sync single repo (e.g. primals/bearDog)
#
# Designed to be fire-and-forget. Safe to run from hooks, cron, or manually.
# Non-destructive: only fast-forwards. Force-push requires --force flag.

set -uo pipefail

ECOPRIMALS_ROOT="${ECOPRIMALS_ROOT:-$HOME/Development/ecoPrimals}"
FORGEJO_URL="http://127.0.0.1:3000"
LOGFILE="${LOGFILE:-/tmp/forgejo_sync.log}"
FORCE=false
STATUS_ONLY=false
SINGLE_REPO=""

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=true ;;
    --status) STATUS_ONLY=true ;;
    --help|-h)
      echo "Usage: $0 [--status] [--force] [repo-path]"
      echo "  --status   Show sync status without pushing"
      echo "  --force    Use --force-with-lease for diverged repos"
      echo "  repo-path  Sync single repo (e.g. primals/bearDog)"
      exit 0
      ;;
    -*) echo "Unknown flag: $arg" >&2; exit 1 ;;
    *) SINGLE_REPO="$arg" ;;
  esac
done

if ! curl -sf "$FORGEJO_URL/api/v1/version" >/dev/null 2>&1; then
  echo "Forgejo not reachable at $FORGEJO_URL — skipping sync"
  exit 0
fi

sync_ok=0
sync_fail=0
sync_skip=0
already_sync=0

sync_repo() {
  local dir="$1"
  local full="$ECOPRIMALS_ROOT/$dir"

  [ -d "$full/.git" ] || return

  if ! git -C "$full" remote | grep -q forgejo; then
    return
  fi

  local branch
  branch=$(git -C "$full" symbolic-ref --short HEAD 2>/dev/null || echo "main")

  local local_ref fg_ref
  local_ref=$(git -C "$full" rev-parse "$branch" 2>/dev/null)
  git -C "$full" fetch forgejo --quiet 2>/dev/null
  fg_ref=$(git -C "$full" rev-parse "forgejo/$branch" 2>/dev/null || echo "none")

  if [ "$local_ref" = "$fg_ref" ]; then
    printf "  %-30s SYNC\n" "$dir"
    ((already_sync++))
    return
  fi

  local ahead
  ahead=$(git -C "$full" rev-list --count "forgejo/$branch".."$branch" 2>/dev/null || echo "?")

  if $STATUS_ONLY; then
    printf "  %-30s ahead +%s\n" "$dir" "$ahead"
    return
  fi

  local push_flags=""
  if $FORCE; then
    push_flags="--force-with-lease"
  fi

  if git -C "$full" push $push_flags forgejo "$branch" >/dev/null 2>&1; then
    printf "  %-30s PUSHED (+%s)\n" "$dir" "$ahead"
    ((sync_ok++))
  else
    printf "  %-30s FAIL (diverged? use --force)\n" "$dir"
    ((sync_fail++))
  fi
}

echo "Forgejo sync — $(date '+%Y-%m-%d %H:%M:%S')"

if [ -n "$SINGLE_REPO" ]; then
  sync_repo "$SINGLE_REPO"
else
  for dir in \
    gardens/cellMembrane gardens/esotericWebb gardens/lithoSpore \
    gardens/projectFOUNDATION gardens/projectNUCLEUS \
    infra/plasmidBin infra/sporePrint infra/wateringHole infra/whitePaper \
    primals/barraCuda primals/bearDog primals/bingoCube primals/biomeOS \
    primals/coralReef primals/loamSpine primals/nestGate primals/petalTongue \
    primals/rhizoCrypt primals/skunkBat primals/songBird primals/sourDough \
    primals/squirrel primals/sweetGrass primals/toadStool \
    springs/airSpring springs/groundSpring springs/healthSpring \
    springs/hotSpring springs/ludoSpring springs/neuralSpring \
    springs/primalSpring springs/wetSpring; do
    sync_repo "$dir"
  done
fi

if ! $STATUS_ONLY; then
  echo ""
  echo "Summary: $sync_ok pushed, $already_sync already synced, $sync_fail failed"
  echo "$(date '+%Y-%m-%d %H:%M:%S') | pushed=$sync_ok synced=$already_sync failed=$sync_fail" >> "$LOGFILE"
fi
