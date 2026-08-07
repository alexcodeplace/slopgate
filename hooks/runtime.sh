#!/usr/bin/env bash
# Resolves the JS runtime the hooks exec. /usr/bin/bun is the workstation's real binary and
# is tried first so the hot hooks skip the PATH bun wrapper; a host without it (buildboxes)
# reaches bun or node through PATH, and every hook payload runs on either. A candidate must
# be named bun or node: a runtime pointed back at a hook would re-enter it on every exec.
# SLOPGATE_RUNTIME pins one explicitly; SLOPGATE_SYSTEM_RUNTIME overrides the absolute
# fast-path candidate (set empty to force PATH resolution).
slopgate_runtime() { # -> path to bun or node on stdout, 1 when neither resolves
  local c
  for c in "${SLOPGATE_RUNTIME:-}" "${SLOPGATE_SYSTEM_RUNTIME-/usr/bin/bun}"; do
    case "${c##*/}" in bun | node) ;; *) continue ;; esac
    [ -x "$c" ] || continue
    printf '%s\n' "$c"
    return 0
  done
  c="$(command -v bun || command -v node)" || return 1
  [ -n "$c" ] || return 1
  printf '%s\n' "$c"
}

slopgate_runtime_or_warn() { # hook-name -> runtime on stdout; warns once, never blocks
  local rt
  rt="$(slopgate_runtime)" || {
    echo "slopgate $1: no bun or node on this host — hook did nothing" >&2
    return 1
  }
  printf '%s\n' "$rt"
}
