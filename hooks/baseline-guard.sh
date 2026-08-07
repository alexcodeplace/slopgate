#!/usr/bin/env bash
# Thin launcher: the guard itself is baseline-guard.mjs. Every parse and regex runs in
# one runtime start because this hook fires on every Bash/Edit/Write call.
HERE="${BASH_SOURCE[0]%/*}"
. "$HERE/runtime.sh"
RUNTIME="$(slopgate_runtime_or_warn baseline-guard)" || exit 0
exec "$RUNTIME" "$HERE/baseline-guard.mjs"
