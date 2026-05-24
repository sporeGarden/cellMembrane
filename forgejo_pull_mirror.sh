#!/usr/bin/env bash
# forgejo_pull_mirror.sh — Convert Forgejo repos to pull mirrors from GitHub
#
# Model: GitHub is authoritative (external membrane). Forgejo is the trailing
# inner membrane mirror. Forgejo pulls FROM GitHub server-side — no dev machine
# involvement. When covalent gates host Forgejo, we invert: Forgejo becomes
# primary, GitHub becomes the push mirror target.
#
# Usage:
#   ./forgejo_pull_mirror.sh --dry-run          # Preview what would change
#   ./forgejo_pull_mirror.sh --migrate          # Delete + recreate as pull mirrors
#   ./forgejo_pull_mirror.sh --sync             # Trigger sync on all mirrors now
#   ./forgejo_pull_mirror.sh --status           # Show mirror status for all repos
#
# Requires: FORGEJO_TOKEN env var, curl, python3
# cellMembrane is excluded — it's inner-only (direct push, not mirrored from GitHub)

set -uo pipefail

FORGEJO_URL="${FORGEJO_URL:-http://127.0.0.1:3000}"
FORGEJO_TOKEN="${FORGEJO_TOKEN:-}"
MIRROR_INTERVAL="${MIRROR_INTERVAL:-8h0m0s}"

if [[ -z "$FORGEJO_TOKEN" ]]; then
  echo "ERROR: FORGEJO_TOKEN required"
  echo "Usage: FORGEJO_TOKEN=<token> $0 [--dry-run|--migrate|--sync|--status]"
  exit 1
fi
AUTH="Authorization: token $FORGEJO_TOKEN"

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
      echo "Usage: FORGEJO_TOKEN=<tok> $0 [--dry-run|--migrate|--sync|--status]"
      echo "  --dry-run   Preview migration without making changes"
      echo "  --migrate   Delete existing repos + recreate as pull mirrors"
      echo "  --sync      Trigger immediate sync on all pull mirrors"
      echo "  --status    Show mirror status for all repos"
      exit 0 ;;
  esac
done

if ! $MIGRATE && ! $SYNC && ! $STATUS && ! $DRY_RUN; then
  echo "ERROR: Specify --dry-run, --migrate, --sync, or --status"
  exit 1
fi

declare -A REPOS
# Format: "forgejo_org/repo_name" -> "github_org/repo_name"
# cellMembrane EXCLUDED — inner-only, not mirrored from GitHub

# gardens (sporeGarden)
REPOS["sporeGarden/projectNUCLEUS"]="sporeGarden/projectNUCLEUS"
REPOS["sporeGarden/projectFOUNDATION"]="sporeGarden/projectFOUNDATION"
REPOS["sporeGarden/lithoSpore"]="sporeGarden/lithoSpore"
REPOS["sporeGarden/esotericWebb"]="sporeGarden/esotericWebb"

# primals (ecoPrimals)
for p in bearDog songBird toadStool nestGate squirrel rhizoCrypt loamSpine \
         sweetGrass biomeOS petalTongue skunkBat barraCuda coralReef \
         bingoCube sourDough; do
  REPOS["ecoPrimals/$p"]="ecoPrimals/$p"
done

# infra (ecoPrimals)
REPOS["ecoPrimals/plasmidBin"]="ecoPrimals/plasmidBin"
REPOS["ecoPrimals/wateringHole"]="ecoPrimals/wateringHole"
REPOS["ecoPrimals/whitePaper"]="ecoPrimals/whitePaper"

# springs (syntheticChemistry)
for s in primalSpring wetSpring hotSpring groundSpring airSpring \
         neuralSpring ludoSpring healthSpring; do
  REPOS["syntheticChemistry/$s"]="syntheticChemistry/$s"
done

# sporePrint is outer-only (GitHub Pages target) but mirror for backup
REPOS["ecoPrimals/sporePrint"]="ecoPrimals/sporePrint"

migrated=0
failed=0
synced=0
skipped=0

for forgejo_path in $(printf '%s\n' "${!REPOS[@]}" | sort); do
  github_path="${REPOS[$forgejo_path]}"
  org="${forgejo_path%%/*}"
  repo="${forgejo_path##*/}"
  github_url="https://github.com/$github_path.git"

  if $STATUS; then
    info=$(curl -sf -H "$AUTH" "$FORGEJO_URL/api/v1/repos/$forgejo_path" 2>/dev/null)
    if [ -z "$info" ]; then
      printf "  %-40s NOT FOUND\n" "$forgejo_path"
      continue
    fi
    is_mirror=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
    interval=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_interval',''))" 2>/dev/null)
    updated=$(echo "$info" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror_updated','')[:19])" 2>/dev/null)
    if [ "$is_mirror" = "True" ]; then
      printf "  %-40s MIRROR  interval=%s  last_sync=%s\n" "$forgejo_path" "$interval" "$updated"
    else
      printf "  %-40s REPO (not a mirror)\n" "$forgejo_path"
    fi
    continue
  fi

  if $SYNC; then
    result=$(curl -sf -X POST -H "$AUTH" "$FORGEJO_URL/api/v1/repos/$forgejo_path/mirror-sync" -w "%{http_code}" -o /dev/null 2>/dev/null)
    if [ "$result" = "200" ]; then
      printf "  %-40s SYNC TRIGGERED\n" "$forgejo_path"
      ((synced++))
    else
      printf "  %-40s SYNC FAILED (HTTP %s)\n" "$forgejo_path" "$result"
      ((failed++))
    fi
    continue
  fi

  # --migrate or --dry-run
  exists=$(curl -sf -o /dev/null -w '%{http_code}' -H "$AUTH" "$FORGEJO_URL/api/v1/repos/$forgejo_path" 2>/dev/null || echo "000")

  if $DRY_RUN; then
    if [ "$exists" = "200" ]; then
      printf "  %-40s WOULD DELETE + RECREATE as mirror from %s\n" "$forgejo_path" "$github_url"
    else
      printf "  %-40s WOULD CREATE mirror from %s\n" "$forgejo_path" "$github_url"
    fi
    continue
  fi

  # Delete existing non-mirror repo
  if [ "$exists" = "200" ]; then
    del_result=$(curl -sf -X DELETE -H "$AUTH" "$FORGEJO_URL/api/v1/repos/$forgejo_path" -w "%{http_code}" -o /dev/null 2>/dev/null)
    if [ "$del_result" != "204" ]; then
      printf "  %-40s DELETE FAILED (HTTP %s)\n" "$forgejo_path" "$del_result"
      ((failed++))
      continue
    fi
    sleep 0.5
  fi

  # Create as pull mirror via migrate API
  migrate_result=$(curl -sf -X POST -H "$AUTH" -H "Content-Type: application/json" \
    "$FORGEJO_URL/api/v1/repos/migrate" \
    -d "{
      \"clone_addr\": \"$github_url\",
      \"repo_name\": \"$repo\",
      \"repo_owner\": \"$org\",
      \"mirror\": true,
      \"mirror_interval\": \"$MIRROR_INTERVAL\",
      \"private\": false,
      \"service\": \"github\"
    }" 2>&1)

  is_mirror=$(echo "$migrate_result" | python3 -c "import json,sys; print(json.load(sys.stdin).get('mirror',False))" 2>/dev/null)
  if [ "$is_mirror" = "True" ]; then
    printf "  %-40s MIGRATED (mirror, interval=%s)\n" "$forgejo_path" "$MIRROR_INTERVAL"
    ((migrated++))
  else
    msg=$(echo "$migrate_result" | python3 -c "import json,sys; print(json.load(sys.stdin).get('message','unknown'))" 2>/dev/null || echo "unknown")
    printf "  %-40s MIGRATE FAILED: %s\n" "$forgejo_path" "$msg"
    ((failed++))
  fi

  sleep 0.3
done

echo ""
if $STATUS; then
  echo "Status check complete."
elif $SYNC; then
  echo "Summary: $synced synced, $failed failed"
elif $MIGRATE || $DRY_RUN; then
  echo "Summary: $migrated migrated, $skipped skipped, $failed failed"
fi
