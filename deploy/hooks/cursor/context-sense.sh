#!/bin/bash
# Context Braid Auto-Sense — sessionStart hook
#
# Runs membrane context.sense --all on session start and injects
# the mesh state as additional context for the agent. This replaces
# the manual "paste a guidance blurb into IDE" pattern.
#
# The agent receives structured TOML braid data covering:
#   - Focus: what's being worked on at each gate
#   - Breadcrumbs: entry points and file locations
#   - Next: upcoming actions
#   - Blockers: what's preventing progress
#   - Notes: standing directives and context

if [ -x "./gardens/cellMembrane/target/release/membrane" ]; then
  MEMBRANE_BIN="./gardens/cellMembrane/target/release/membrane"
elif [ -x "./gardens/cellMembrane/target/debug/membrane" ]; then
  MEMBRANE_BIN="./gardens/cellMembrane/target/debug/membrane"
else
  MEMBRANE_BIN="$(command -v membrane 2>/dev/null)"
fi

if [ ! -x "$MEMBRANE_BIN" ]; then
  echo '{"additional_context": "membrane binary not found — run cargo build --bin membrane in gardens/cellMembrane"}'
  exit 0
fi

CONTEXT_OUTPUT=$("$MEMBRANE_BIN" context.sense --all 2>/dev/null)
POTENTIAL_OUTPUT=$("$MEMBRANE_BIN" potential.sense 2>/dev/null)

if [ -z "$CONTEXT_OUTPUT" ] && [ -z "$POTENTIAL_OUTPUT" ]; then
  echo '{"additional_context": "No context braids or pending impulses (resting state)."}'
  exit 0
fi

COMBINED="=== CONTEXT BRAIDS (mesh state) ===\n${CONTEXT_OUTPUT}\n\n=== PENDING IMPULSES ===\n${POTENTIAL_OUTPUT}"

# Escape for JSON
ESCAPED=$(printf '%s' "$COMBINED" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))' 2>/dev/null)

if [ -z "$ESCAPED" ]; then
  ESCAPED=$(printf '%s' "$COMBINED" | sed 's/\\/\\\\/g; s/"/\\"/g; s/\n/\\n/g' | tr '\n' ' ')
  ESCAPED="\"$ESCAPED\""
fi

echo "{\"additional_context\": $ESCAPED}"
exit 0
