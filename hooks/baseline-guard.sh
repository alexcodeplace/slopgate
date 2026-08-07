#!/usr/bin/env bash
# Thin launcher: the guard itself is baseline-guard.mjs. Every parse and regex runs in
# one runtime start because this hook fires on every Bash/Edit/Write call.
exec /usr/bin/bun "${BASH_SOURCE[0]%/*}/baseline-guard.mjs"
