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

echo "== commit-hook (0=allow/skip; no .slopgate config in sandbox HOME) =="
chk commit-hook.sh 0 "non-commit Bash fast-skips"        '{"tool_name":"Bash","tool_input":{"command":"ls -la"}}'

echo "== session-start (0, records model, no crash) =="
chk session-start.sh 0 "session-start records model"     '{"model":"opus","session_id":"s1","cwd":"'"$PWD"'"}'

echo "---- pass=$pass fail=$fail ----"
[ "$fail" -eq 0 ]
