#!/usr/bin/env bash
# Slopgate PostToolUse hook — single-file scan after Edit/Write.
# Exit 2 returns either policy findings or an incomplete check to the agent.
# Project configuration, not this hook, owns file and language selection.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "$HERE/runtime.sh"
RUNTIME="$(slopgate_runtime_or_warn edit-hook)" || exit 0
TOOL_JSON=$(cat)
FILE=$("$RUNTIME" -e "
let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{try{process.stdout.write(JSON.parse(d).tool_input?.file_path||'')}catch{process.stdout.write('')}});" <<< "$TOOL_JSON" 2>/dev/null) || exit 0
[ -n "$FILE" ] || exit 0
# Skip fixture files — they are intentional violation examples for slopgate self-test.
case "$FILE" in */.slopgate/fixtures/*|*/slopgate/*/fixtures/*) exit 0 ;; esac

ROOT=$(git -C "$(dirname "$FILE")" rev-parse --show-toplevel 2>/dev/null) || exit 0
CONFIG="$ROOT/.slopgate/config.toml"
[ -f "$CONFIG" ] || exit 0

OUT=$("$RUNTIME" "$HERE/../bin/slopgate" --file "$FILE" --config "$CONFIG" 2>&1)
STATUS=$?
if [ "$STATUS" -ne 0 ]; then echo "$OUT" >&2; exit 2; fi
exit 0
