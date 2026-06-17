#!/usr/bin/env bash
# Slopgate UserPromptSubmit hook — optional prompt meta advisory.
# FAIL-OPEN: missing config, disabled config, errors, or timeout -> exit 0.
HOOK_JSON=$(cat)
CWD=$(node -e '
let d = "";
process.stdin.on("data", c => d += c).on("end", () => {
  try {
    const j = JSON.parse(d);
    process.stdout.write(j.cwd || process.cwd());
  } catch {
    process.stdout.write(process.cwd());
  }
});
' <<< "$HOOK_JSON" 2>/dev/null) || exit 0
[ -n "$CWD" ] || CWD="$PWD"

ROOT=$(git -C "$CWD" rev-parse --show-toplevel 2>/dev/null || printf '%s' "$CWD")
CONFIG="$ROOT/.slopgate/config.toml"
[ -f "$CONFIG" ] || exit 0

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
run_slopgate() {
  if command -v timeout >/dev/null 2>&1; then
    timeout 5 node "$HERE/../bin/slopgate" prompt-meta --config "$CONFIG"
  else
    node "$HERE/../bin/slopgate" prompt-meta --config "$CONFIG"
  fi
}

OUT=$(printf '%s' "$HOOK_JSON" | run_slopgate 2>/dev/null) || exit 0
[ -n "$OUT" ] && printf '%s\n' "$OUT"
exit 0
