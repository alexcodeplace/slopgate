# slop-gate engine — code-quality pass

**Date:** 2026-06-10
**Scope:** "Everything incl. DRY/structure" — correctness bug + redundancy/perf + DRY/cleanup.
**Constraint:** Behavior-preserving. All 13 existing test files must stay green; `self-test` unchanged. No public-API churn beyond what's named below.

## Problem

Engine works and is tested, but review surfaced one latent correctness bug, three redundancy/perf defects, and three DRY/structure smells. None are spec-compliance failures — this is purely *how* it's built.

## Findings → Changes

### C1 — `/g`-flag regex statefulness (CORRECTNESS)
`src/regex-engine.mjs` calls `re.test(line)` in a per-line loop. A `RegExp` built with the `g` flag is stateful: `lastIndex` advances across `.test()` calls, so a `/g`-flagged rule matches only every *other* line. Confirmed: `/x/g`.test → `true,false,true`. No baseline rule sets `g` today, so it's latent — but a future project rule with `g` would silently half-fire.

**Fix:** the line-scan engine must use non-global matching. When compiling a rule's regex for line scanning, strip `g` from `flags` (and `y`, which is also stateful). Single source of truth: a tiny `compileLineRegex(pattern, flags)` helper inside `regex-engine.mjs` that removes `g`/`y` before `new RegExp`. Self-test (`selftest.mjs`) uses `.test(canary)` once so is unaffected, but should use the same helper for consistency so canary behavior matches engine behavior exactly.

### C2 — double file enumeration (PERF/REDUNDANCY)
`gate.mjs:collectViolations` calls `listSourceFiles(config, opts)` (line 18), then `runPatternScan(config, opts)` calls `listSourceFiles` **again** internally. In `staged` mode that's two `git diff --cached` execs; in `full` mode two directory walks.

**Fix:** thread the already-computed `files` into the regex layer. `runPatternScan(config, files, opts)` and `collectRegexViolations(config, files, findings)` take the file list from the caller instead of re-enumerating. `collectViolations` passes its single `files` array down. (ast layer already receives `files` — leave as is.)

### C3 — two-pass + per-pattern re-reads in regex engine (PERF/REDUNDANCY)
Two layers of waste:
- `searchPattern` reads **every file** fresh **for every pattern** (P patterns × M files reads).
- `runPatternScan` computes `byFile` (file→matched line numbers) then **throws it away**; `collectRegexViolations` re-reads each file *and* re-runs the regex to recover the same line numbers.

**Fix:** restructure to read each file **once**. Single pass:
1. For each file: read once, split lines, test all patterns against each line, record `(patternId → [{line, lineText}])` plus per-pattern hit-file set.
2. Apply the `minFiles` threshold per pattern (needs all files counted — keep this as a post-pass over the collected hit sets, same semantics as today).
3. Expand surviving patterns' recorded hits into violations (line, lineHash, text) — no re-read, no re-test.

`fileMode` skip of `minFiles>1` rules and `excludeGlobs` per-pattern filtering preserved. Public surface: `runPatternScan` / `collectRegexViolations` may be merged into one `scanRegex(config, files)` returning violations directly, since no caller uses `findings` between them (verify: only `gate.mjs` calls both, back-to-back). Keep `pathMatchesGlobs` and `lineHash` usage identical.

### D1 — duplicated gate-filter logic (DRY)
`cli.mjs:snapshotViolations` re-implements the severity-allow + suppression filter that `gate.mjs:runGate` already does (lines 58–64).

**Fix:** extract `applyGateFilters(violations, config, mode)` in `gate.mjs` (returns severity-allowed, non-suppressed violations; emits the malformed-suppressions warning once). `runGate` and `cli.snapshotViolations` both call it. Baseline/ratchet filtering stays in `runGate` only (snapshot intentionally pre-baseline).

### D2 — hardcoded `ENGINE_ROOT` duplicated (DRY/magic constant)
`'/home/user/Projects/slop-gate'` is hardcoded in **both** `install-hooks.mjs` and `init.mjs`. It's a single-machine personal tool (acknowledged), but the path is derivable.

**Fix:** compute `ENGINE_ROOT` once from `import.meta.url` in `install-hooks.mjs` (engine root = two dirs up from `src/`), export it, and have `init.mjs` import it. Eliminates the literal entirely and survives a repo move. `COMMIT_HOOK`/`EDIT_HOOK` in `init.mjs` derive from the imported root.

### D3 — convoluted config dedupe (CLARITY)
`config.mjs` builds a `byId` Map (last-wins) **and** a separate `order`/`seen` array to reconstruct first-occurrence order, then maps one through the other — 8 lines for what Map already guarantees.

**Fix:** `Map.set` on an existing key keeps first-insertion position and updates the value (confirmed). Collapse to: `const byId = new Map(); for (const p of patterns) byId.set(p.id, p); const dedupedPatterns = [...byId.values()];`. Same result (first-position, last-wins value).

## Out of scope (YAGNI)
- No new features, no config keys, no severity changes.
- `init.mjs` size (371 lines) left intact — it's cohesive scaffolding; splitting adds import surface for no current consumer. (Deletion test: detection helpers have one caller — `runInit` — and their complexity would just move, not shrink.)
- ast-engine temp-dir cleanup was raised but is genuinely small; included as a low-risk extra: `mkdtempSync` sgconfig dir is never removed. **Fix:** wrap the scan in `try/finally` and `rmSync(dir, {recursive,force})`, mirroring `jscpd.mjs`. (Keeps the pattern consistent across the two temp-dir users.)

## Files changed
- `src/regex-engine.mjs` — C1 helper, C2 file-list param, C3 single-pass restructure.
- `src/gate.mjs` — C2 pass `files` down, D1 extract+use `applyGateFilters`.
- `src/cli.mjs` — D1 use `applyGateFilters`.
- `src/selftest.mjs` — C1 use shared `compileLineRegex`.
- `src/install-hooks.mjs` — D2 derive+export `ENGINE_ROOT`.
- `src/init.mjs` — D2 import `ENGINE_ROOT`.
- `src/config.mjs` — D3 collapse dedupe.
- `src/ast-engine.mjs` — temp-dir cleanup (try/finally).

## Testing
- All 13 `*.test.mjs` must pass unchanged (no test edits expected; if a test asserts the old `runPatternScan(config, opts)` arity, update the call, not the assertion).
- `npm run self-test` exit 0, same OK/FAIL lines.
- Add one regex-engine assertion: a rule with `flags:'g'` must match **every** matching line (locks C1).
- Manual: `slop-gate --staged` and `--file` on this repo produce identical violation output pre/post.

## Architecture Decisions
- **Merge `runPatternScan`+`collectRegexViolations` → `scanRegex` (accepted):** deletion test — the seam between "find hit files" and "expand to violations" hid nothing; `gate.mjs` is the only caller and uses them back-to-back. Collapsing removes a re-read and a re-test. minFiles still computed before expansion, inside the merged function.
- **`applyGateFilters` extraction (accepted):** earns its boundary by the deletion test — same 6 lines live in two callers today; centralizing fixes the one-warning-per-load contract in one place.
- **`compileLineRegex` helper (accepted, shallow but justified):** shallow seam, but it's the *single enforcement point* for "engine never uses stateful flags" — a correctness invariant worth naming over inlining.
- **Split `init.mjs` (rejected):** detection helpers have exactly one caller; complexity would scatter, not shrink. Single-adapter — no second consumer exists.
- **`ENGINE_ROOT` from `import.meta.url` (accepted):** removes a duplicated magic literal; two-way door (still a single-machine assumption, just not hardcoded twice).
