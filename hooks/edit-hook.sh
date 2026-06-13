#!/usr/bin/env bash
# Slopgate PostToolUse hook — single-file scan after Edit/Write.
# Exit 2 → stderr feeds back into the agent turn. FAIL-OPEN: any error/timeout → exit 0.
TOOL_JSON=$(cat)
FILE=$(node -e "
let d='';process.stdin.on('data',c=>d+=c).on('end',()=>{try{process.stdout.write(JSON.parse(d).tool_input?.file_path||'')}catch{process.stdout.write('')}});" <<< "$TOOL_JSON" 2>/dev/null) || exit 0
[ -n "$FILE" ] || exit 0
case "$FILE" in *.test.ts|*.test.tsx) exit 0 ;; *.ts|*.tsx|*.astro) ;; *) exit 0 ;; esac
# Skip fixture files — they are intentional violation examples for slopgate self-test.
case "$FILE" in */.slopgate/fixtures/*|*/slopgate/*/fixtures/*) exit 0 ;; esac

ROOT=$(git -C "$(dirname "$FILE")" rev-parse --show-toplevel 2>/dev/null) || exit 0
CONFIG="$ROOT/.slopgate/config.toml"
[ -f "$CONFIG" ] || exit 0

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=$(timeout 5 node "$HERE/../bin/slopgate" --file "$FILE" --config "$CONFIG" 2>&1)
[ "$?" -eq 1 ] && { echo "$OUT" >&2; exit 2; }
exit 0