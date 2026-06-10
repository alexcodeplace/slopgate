#!/usr/bin/env bash
# Slopgate PreToolUse hook — runs --staged before a git commit. Exit 1 → commit blocked.
TOOL_JSON=$(cat)
CMD=$(node -e "
let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{try{process.stdout.write(JSON.parse(d).tool_input?.command||'')}catch{process.stdout.write('')}});" <<< "$TOOL_JSON" 2>/dev/null)
echo "$CMD" | grep -qE 'git[[:space:]]+commit' || exit 0

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
CONFIG="$ROOT/.slopgate/config.mjs"
[ -f "$CONFIG" ] || exit 0
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec node "$HERE/../bin/slopgate" --staged --config "$CONFIG"