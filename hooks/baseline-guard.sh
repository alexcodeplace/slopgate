#!/usr/bin/env bash
# Fast-path launcher: every trigger pattern in baseline-guard.mjs requires the literal
# "slopgate" somewhere in the payload (.slopgate/ paths, `slopgate baseline` commands).
# The common call carries no such substring, so a pure-bash check answers it without a
# runtime start (measured: ~67ms bun boot -> ~3ms). Payload with the literal falls
# through to the real guard, verdict semantics unchanged.
payload=$(cat)
case "$payload" in
  *slopgate*) printf '%s' "$payload" | exec /usr/bin/bun "${BASH_SOURCE[0]%/*}/baseline-guard.mjs" ;;
  *) exit 0 ;;
esac
