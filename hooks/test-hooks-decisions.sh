#!/usr/bin/env bash
# Decision regression tests for the agent-facing hooks (baseline-guard, commit-hook,
# session-start). Guards the block/allow contract across interpreter changes.
# HOME is sandboxed so stats/session writes never touch the real ~/.slopgate.
# Run: bash hooks/test-hooks-decisions.sh   (exit 0 = all pass)
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT
export HOME="$SANDBOX"

pass=0; fail=0
chk(){ # script  expected_rc  label  payload
  printf '%s' "$4" | bash "$HERE/$1" >/dev/null 2>&1
  local rc=$?
  if [ "$rc" -eq "$2" ]; then echo "PASS ($rc) $3"; pass=$((pass+1))
  else echo "FAIL (got $rc want $2) $3"; fail=$((fail+1)); fi
}

echo "== baseline-guard (2=block, 0=allow) =="
chk baseline-guard.sh 2 "Edit baseline.json blocked"     '{"tool_name":"Edit","tool_input":{"file_path":"/x/.slopgate/baseline.json"}}'
chk baseline-guard.sh 2 "Edit suppressions.json blocked" '{"tool_name":"Edit","tool_input":{"file_path":"/x/.slopgate/suppressions.json"}}'
chk baseline-guard.sh 0 "Edit normal file allowed"       '{"tool_name":"Edit","tool_input":{"file_path":"/x/src/app.ts"}}'
chk baseline-guard.sh 2 "Bash slopgate baseline blocked" '{"tool_name":"Bash","tool_input":{"command":"slopgate baseline --update"}}'
BJSON=".slopgate/baseline.json"
chk baseline-guard.sh 2 "Bash rm baseline blocked"       "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"rm $BJSON\"}}"
chk baseline-guard.sh 0 "Bash benign allowed"            '{"tool_name":"Bash","tool_input":{"command":"ls -la"}}'
BCMD="slopgate ""baseline"
chk baseline-guard.sh 2 "Write baseline.json blocked"    '{"tool_name":"Write","tool_input":{"file_path":"/x/.slopgate/baseline.json"}}'
chk baseline-guard.sh 2 "Bash npx wrapper blocked"       "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"npx $BCMD\"}}"
chk baseline-guard.sh 2 "Bash node bin wrapper blocked"  "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"node ./dist/bin/$BCMD --update\"}}"
chk baseline-guard.sh 2 "Bash mv baseline blocked"       "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"mv $BJSON /tmp/x\"}}"
chk baseline-guard.sh 0 "Read is not gated"              '{"tool_name":"Read","tool_input":{"file_path":"/x/.slopgate/baseline.json"}}'
chk baseline-guard.sh 0 "empty payload allowed"          ''

# A block must still emit the operator-facing reason and the bypass-attempt stats row.
mkdir -p "$SANDBOX/.slopgate/sessions"
skey=$(printf '%s' "$PWD" | sha256sum | cut -c1-16)
printf '{"model":"opus","sessionId":"s1"}' > "$SANDBOX/.slopgate/sessions/$skey.json"
blk_err=$(printf '%s' '{"tool_name":"Edit","tool_input":{"file_path":"/x/.slopgate/baseline.json"}}' | bash "$HERE/baseline-guard.sh" 2>&1 >/dev/null)
if [[ "$blk_err" == *"SLOPGATE GUARD"* ]]; then echo "PASS (0) block prints the guard reason"; pass=$((pass+1));
else echo "FAIL block prints the guard reason: $blk_err"; fail=$((fail+1)); fi
if grep -q '"ruleId":"baseline-tamper"' "$SANDBOX/.slopgate/stats.jsonl" 2>/dev/null; then
  echo "PASS (0) block records a bypass-attempt stats row"; pass=$((pass+1));
else echo "FAIL block records a bypass-attempt stats row"; fail=$((fail+1)); fi

echo "== commit-hook (0=allow/skip; no .slopgate config in sandbox HOME) =="
chk commit-hook.sh 0 "non-commit Bash fast-skips"        '{"tool_name":"Bash","tool_input":{"command":"ls -la"}}'

echo "== session-start (0, records model, no crash) =="
chk session-start.sh 0 "session-start records model"     '{"model":"opus","session_id":"s1","cwd":"'"$PWD"'"}'

echo "---- pass=$pass fail=$fail ----"
[ "$fail" -eq 0 ]
