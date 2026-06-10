#!/usr/bin/env bash
# Slop-gate PreToolUse hook — runs --staged before a git commit. Exit 1 → commit blocked.
TOOL_JSON=$(cat)
CMD=$(node -e "
let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{try{process.stdout.write(JSON.parse(d).tool_input?.command||'')}catch{process.stdout.write('')}});" <<< "$TOOL_JSON" 2>/dev/null)
echo "$CMD" | grep -qE '(git[[:space:]]+commit|commit_push\.sh|deploy\.sh)' || exit 0

ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
CONFIG="$ROOT/.slop-gate/config.mjs"
[ -f "$CONFIG" ] || exit 0
exec node /home/user/Projects/slop-gate/bin/slop-gate --staged --config "$CONFIG"