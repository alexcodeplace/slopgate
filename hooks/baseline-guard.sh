#!/usr/bin/env bash
# Fast-path launcher: every trigger pattern in baseline-guard.mjs requires the literal
# "slopgate" somewhere in the payload (.slopgate/ paths, `slopgate baseline` commands).
# The common call carries no such substring, so a pure-bash check answers it without a
# runtime start (measured: ~67ms runtime boot -> ~5ms). Payload with the literal falls
# through to the real guard, verdict semantics unchanged.
payload=$(</dev/stdin)
case "$payload" in
  *slopgate*) ;;
  *) exit 0 ;;
esac
HERE="${BASH_SOURCE[0]%/*}"
. "$HERE/runtime.sh"
RUNTIME="$(slopgate_runtime_or_warn baseline-guard)" || exit 0
printf '%s' "$payload" | "$RUNTIME" "$HERE/baseline-guard.mjs"
