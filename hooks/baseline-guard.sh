#!/usr/bin/env bash
# Slopgate baseline-guard — PreToolUse hook blocking agent bypass of the gate.
# Covers: Edit/Write to baseline.json/suppressions.json, Bash rm/mv on baseline*,
# Bash `slopgate baseline` in any form.
# Exit 2 = block the tool use (PreToolUse protocol). Exit 0 = allow. Always fail-open on errors.

set -euo pipefail
INPUT=$(cat)

tool_name=$(node -e "try{const j=JSON.parse(process.argv[1]);process.stdout.write(j.tool_name||'')}catch{}" "$INPUT" 2>/dev/null || echo '')
tool_input=$(node -e "try{const j=JSON.parse(process.argv[1]);process.stdout.write(JSON.stringify(j.tool_input||{}))}catch{}" "$INPUT" 2>/dev/null || echo '{}')

blocked=0
reason=''

case "$tool_name" in
  Edit|Write)
    file_path=$(node -e "try{const j=JSON.parse(process.argv[1]);process.stdout.write(j.file_path||'')}catch{}" "$tool_input" 2>/dev/null || echo '')
    if echo "$file_path" | grep -qE '\.slopgate/(baseline|suppressions)\.json$'; then
      blocked=1
      reason="direct write to $file_path blocked — edit baseline/suppressions via slopgate CLI only"
    fi
    ;;
  Bash)
    cmd=$(node -e "try{const j=JSON.parse(process.argv[1]);process.stdout.write(j.command||'')}catch{}" "$tool_input" 2>/dev/null || echo '')
    # Block: slopgate baseline (create or --update)
    if echo "$cmd" | grep -qE '(^|[;&|])\s*(node\s+[^ ]+/bin/slopgate|slopgate)\s+baseline(\s|$)'; then
      blocked=1
      reason="slopgate baseline blocked — agent cannot update/create baseline; run in your own terminal"
    fi
    # Block: rm/mv targeting .slopgate/baseline*
    if echo "$cmd" | grep -qE '(rm|mv)\s+.*\.slopgate/baseline'; then
      blocked=1
      reason="rm/mv of .slopgate/baseline blocked — baseline integrity guard"
    fi
    ;;
esac

if [ "$blocked" -eq 1 ]; then
  echo "⛔ SLOPGATE GUARD: $reason" >&2
  # Write bypass-attempt stats row (fail-open)
  ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
  node -e "
const crypto=require('crypto'),fs=require('fs'),path=require('path'),os=require('os');
const root=process.argv[1], reason=process.argv[2];
try {
  const key=crypto.createHash('sha256').update(root).digest('hex').slice(0,16);
  const sess=JSON.parse(fs.readFileSync(path.join(os.homedir(),'.slopgate','sessions',key+'.json'),'utf8'));
  const row=JSON.stringify({ts:new Date().toISOString(),project:path.basename(root),projectPath:root,
    model:sess.model||'unknown',sessionId:sess.sessionId||null,mode:'bypass-attempt',
    ruleId:'baseline-tamper',severity:'critical',category:'security',engine:'bypass-attempt',
    file:null,line:null,reason});
  const gp=path.join(os.homedir(),'.slopgate','stats.jsonl');
  fs.appendFileSync(gp,row+'\n');
} catch { /* fail-open */ }
" "$ROOT" "$reason" 2>/dev/null || true
  exit 2
fi

exit 0
