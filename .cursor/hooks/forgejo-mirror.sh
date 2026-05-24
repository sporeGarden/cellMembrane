#!/usr/bin/env bash
# Cursor afterShellExecution hook — async Forgejo mirror after git push
#
# Fires after any `git push` to a non-forgejo remote. Detects the repo,
# determines the branch, and pushes to forgejo in the background.
# Fire-and-forget: failures are logged but never block the agent.

set -uo pipefail

input=$(cat)
command=$(echo "$input" | python3 -c "import json,sys; print(json.load(sys.stdin).get('shell_command',json.load(open('/dev/stdin') if False else sys.stdin).get('command','')))" 2>/dev/null || echo "")
exit_code=$(echo "$input" | python3 -c "import json,sys; print(json.load(sys.stdin).get('exit_code',1))" 2>/dev/null || echo "1")
cwd=$(echo "$input" | python3 -c "import json,sys; print(json.load(sys.stdin).get('cwd',''))" 2>/dev/null || echo "")

# Only mirror successful pushes
if [ "$exit_code" != "0" ]; then
  exit 0
fi

# Skip if already pushing to forgejo
if echo "$command" | grep -q 'push.*forgejo'; then
  exit 0
fi

# Skip if Forgejo unreachable
if ! curl -sf --connect-timeout 2 "http://127.0.0.1:3000/api/v1/version" >/dev/null 2>&1; then
  exit 0
fi

# Determine the git repo directory
repo_dir=""
if [ -n "$cwd" ] && [ -d "$cwd/.git" ]; then
  repo_dir="$cwd"
elif [ -n "$cwd" ]; then
  repo_dir=$(git -C "$cwd" rev-parse --show-toplevel 2>/dev/null || echo "")
fi

if [ -z "$repo_dir" ] || [ ! -d "$repo_dir/.git" ]; then
  exit 0
fi

# Check if this repo has a forgejo remote
if ! git -C "$repo_dir" remote | grep -q forgejo; then
  exit 0
fi

branch=$(git -C "$repo_dir" symbolic-ref --short HEAD 2>/dev/null || echo "main")

# Async push — fire and forget
(
  git -C "$repo_dir" push forgejo "$branch" >/dev/null 2>&1 || true
) &
disown

exit 0
