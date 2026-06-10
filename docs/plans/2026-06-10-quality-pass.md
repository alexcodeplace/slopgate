# slop-gate Quality Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use /ship (recommended) or /executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate one latent regex-statefulness bug, two redundant-work paths, and three DRY/clarity smells in the slop-gate engine — behavior-preserving.

**Architecture:** Pure refactor of an existing, fully-tested Node ESM CLI. Single-pass file scanning replaces a per-pattern re-read + two-pass expand; shared helpers replace duplicated filter logic and a duplicated magic path constant. No new config keys, no API additions beyond named helpers.

**Tech Stack:** Node ≥18 ESM (`.mjs`), `node:test`-style hand-rolled assertion scripts (each `src/**/*.test.mjs` is an executable that exits non-zero on failure), no build step.

---

## Wave Plan

| Wave | Tasks | Files touched | Safe to parallelize? |
|------|-------|---------------|----------------------|
| 1 | Task 1, Task 2, Task 3, Task 4 | `config.mjs` / `install-hooks.mjs`+`init.mjs` / `ast-engine.mjs` / `gate.mjs`+`cli.mjs` | ✅ zero file overlap between the four |
| 2 | Task 5 | `regex-engine.mjs`, `regex-engine.test.mjs`, `gate.mjs`, `selftest.mjs` | single task (touches `gate.mjs` after Task 4) |

**Conventions for every commit step:** stage only the exact `src/` paths named. Never `git add docs/`, `git add .`, the spec, or this plan. Tests live in `src/**/*.test.mjs` and ARE tracked — staging them is correct.

**Baseline (run once before starting):** `for f in $(find src -name '*.test.mjs'); do node "$f" >/dev/null 2>&1 && echo "ok $f" || echo "FAIL $f"; done` → all `ok`. And `npm run self-test` → exit 0.

---

### Task 1: Collapse the config dedupe

**Wave:** 1
**Blocks:** —
**Blocked by:** —

**Files:**
- Modify: `src/config.mjs:43-51`

**Context:** `Map.set` on an existing key keeps the key's first-insertion position and updates its value (verified: `new Map(); set('a',1); set('b',2); set('a',9)` → `[['a',9],['b',2]]`). The current code builds a `byId` map AND a separate `order`/`seen` array to reconstruct first-occurrence order — the Map already guarantees it. Last-wins value = project rule overrides baseline on id collision; first-position order preserved. Behavior identical.

- [ ] **Step 1: Replace the dedupe block**

In `src/config.mjs`, replace these lines (currently 43–51):

```js
  // dedupe by id (last-wins: project overrides baseline on collision)
  const byId = new Map();
  for (const p of patterns) byId.set(p.id, p);
  const order = [];
  const seen = new Set();
  for (const p of patterns) {
    if (!seen.has(p.id)) { order.push(p.id); seen.add(p.id); }
  }
  const dedupedPatterns = order.map((id) => byId.get(id));
```

with:

```js
  // dedupe by id (last-wins value, first-occurrence order — both guaranteed by Map)
  const byId = new Map();
  for (const p of patterns) byId.set(p.id, p);
  const dedupedPatterns = [...byId.values()];
```

- [ ] **Step 2: Run the config dedupe coverage**

Run: `node src/config.checkers.test.mjs && node src/gate.tier.test.mjs && node src/gate.e2e.test.mjs`
Expected: each prints its `PASS:` lines and exits 0 (no `FAIL`).

- [ ] **Step 3: Self-test still green**

Run: `npm run self-test`
Expected: exit 0, `OK` lines for each rule, no `FAIL`.

- [ ] **Step 4: Commit**

```bash
git add src/config.mjs
git commit -m "refactor(config): collapse dedupe — Map already gives first-pos/last-win"
```

---

### Task 2: Derive `ENGINE_ROOT` instead of hardcoding it twice

**Wave:** 1
**Blocks:** —
**Blocked by:** —

**Files:**
- Modify: `src/install-hooks.mjs:7-11`
- Modify: `src/init.mjs:1-19`

**Context:** `'/home/user/Projects/slop-gate'` is hardcoded in both files. The engine root is two directories up from `src/` (i.e. up from the module's own dir, then up once more). Compute it once in `install-hooks.mjs` from `import.meta.url`, export it, import it in `init.mjs`. Single-machine assumption stays, but the literal lives in exactly one place and survives a repo move.

- [ ] **Step 1: Compute + export `ENGINE_ROOT` in install-hooks.mjs**

In `src/install-hooks.mjs`, add `dirname` + `fileURLToPath` to imports and replace the hardcoded constant. Current import line (9) and constant (11):

```js
import { join, isAbsolute } from 'node:path';

const ENGINE_ROOT = '/home/user/Projects/slop-gate';
```

becomes:

```js
import { join, isAbsolute, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// engine root = parent of src/ (this file lives in src/)
export const ENGINE_ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
```

- [ ] **Step 2: Import it in init.mjs**

In `src/init.mjs`, change the import block (lines 1–5) to also import `ENGINE_ROOT`, and delete the local hardcoded constant (line 17). The import of `installPreCommitHook` (line 5):

```js
import { installPreCommitHook } from './install-hooks.mjs';
```

becomes:

```js
import { installPreCommitHook, ENGINE_ROOT } from './install-hooks.mjs';
```

and delete line 17 entirely:

```js
const ENGINE_ROOT = '/home/user/Projects/slop-gate';
```

(Leave `COMMIT_HOOK` / `EDIT_HOOK` lines 18–19 unchanged — they already interpolate `ENGINE_ROOT`, now the imported one.)

- [ ] **Step 3: Verify the install-hooks + init tests pass**

Run: `node src/install-hooks.test.mjs && node src/init.test.mjs`
Expected: both print `PASS:` lines and exit 0.

- [ ] **Step 4: Verify the derived path is correct**

Run: `node -e "import('./src/install-hooks.mjs').then(m=>console.log(m.ENGINE_ROOT))"`
Expected: prints `/home/user/Projects/slop-gate` (the real repo root).

- [ ] **Step 5: Commit**

```bash
git add src/install-hooks.mjs src/init.mjs
git commit -m "refactor: derive ENGINE_ROOT from import.meta.url, drop duplicated literal"
```

---

### Task 3: Clean up the ast-grep temp-dir leak

**Wave:** 1
**Blocks:** —
**Blocked by:** —

**Files:**
- Modify: `src/ast-engine.mjs:6-9, 34-72`

**Context:** `runAstGrepScan` creates a temp dir via `mkdtempSync` for the synthesized `sgconfig.yml` and never removes it — every scan leaks a dir under `tmpdir()`. `jscpd.mjs` already wraps its temp dir in `try/finally` + `rmSync`; mirror that pattern. All existing early-`return`s after the `mkdtempSync` line must move inside the `try` so the `finally` always runs.

- [ ] **Step 1: Add `rmSync` to the fs import**

In `src/ast-engine.mjs` line 7:

```js
import { existsSync, writeFileSync, mkdtempSync } from 'node:fs';
```

becomes:

```js
import { existsSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
```

- [ ] **Step 2: Wrap the post-mkdtemp body in try/finally**

In `src/ast-engine.mjs`, the block from the `mkdtempSync` line (34) through the final `return` (72) currently is:

```js
  const dir = mkdtempSync(join(tmpdir(), 'slopgate-sg-'));
  const sgConfig = join(dir, 'sgconfig.yml');
  writeFileSync(sgConfig, 'ruleDirs:\n' + ruleDirs.map((d) => `  - ${d}`).join('\n') + '\n');

  const targets = files === null ? config.rootsRel : (opts.rawTargets ? files : files.filter((f) => /\.(ts|tsx)$/.test(f)));
  if (files !== null && targets.length === 0) return { available: true, violations: [], errors: [] };

  const res = spawnSync(bin, ['scan', '--config', sgConfig, '--json', ...targets], {
    encoding: 'utf8', cwd: config.repoRoot, maxBuffer: 32 * 1024 * 1024, timeout: 60_000,
  });
  if (res.error || res.stdout == null) {
    return { available: false, violations: [], errors: [`ast-grep failed: ${res.error || res.stderr?.slice(0, 300)}`] };
  }
  let matches;
  try { matches = JSON.parse(res.stdout); } catch (e) {
    return { available: true, violations: [], errors: [`ast-grep JSON parse error: ${e}`] };
  }
  const violations = [];
  const errors = [];
  if (res.stderr && /error/i.test(res.stderr) && !/error\(s\) found in code/i.test(res.stderr)) {
    errors.push(`ast-grep stderr: ${res.stderr.slice(0, 500)}`);
  }
  for (const m of matches) {
    let meta = {};
    try { meta = JSON.parse(m.note || '{}'); } catch { errors.push(`rule ${m.ruleId}: note is not valid JSON`); }
    const firstLine = (m.lines || '').split('\n')[0];
    violations.push({
      id: m.ruleId,
      severity: meta.severity || (m.severity === 'error' ? 'high' : 'medium'),
      category: meta.category || 'convention',
      file: m.file,
      line: (m.range?.start?.line ?? 0) + 1,
      fullLine: firstLine,
      text: firstLine.trim().slice(0, 90),
      resolution: meta.resolution || m.message || '',
      engine: 'ast',
    });
  }
  return { available: true, violations, errors };
```

Replace it with the same logic wrapped so the dir is always removed (note: keep the inner `try/catch` for `JSON.parse(res.stdout)` as-is; only the outer `try/finally` is new):

```js
  const dir = mkdtempSync(join(tmpdir(), 'slopgate-sg-'));
  try {
    const sgConfig = join(dir, 'sgconfig.yml');
    writeFileSync(sgConfig, 'ruleDirs:\n' + ruleDirs.map((d) => `  - ${d}`).join('\n') + '\n');

    const targets = files === null ? config.rootsRel : (opts.rawTargets ? files : files.filter((f) => /\.(ts|tsx)$/.test(f)));
    if (files !== null && targets.length === 0) return { available: true, violations: [], errors: [] };

    const res = spawnSync(bin, ['scan', '--config', sgConfig, '--json', ...targets], {
      encoding: 'utf8', cwd: config.repoRoot, maxBuffer: 32 * 1024 * 1024, timeout: 60_000,
    });
    if (res.error || res.stdout == null) {
      return { available: false, violations: [], errors: [`ast-grep failed: ${res.error || res.stderr?.slice(0, 300)}`] };
    }
    let matches;
    try { matches = JSON.parse(res.stdout); } catch (e) {
      return { available: true, violations: [], errors: [`ast-grep JSON parse error: ${e}`] };
    }
    const violations = [];
    const errors = [];
    if (res.stderr && /error/i.test(res.stderr) && !/error\(s\) found in code/i.test(res.stderr)) {
      errors.push(`ast-grep stderr: ${res.stderr.slice(0, 500)}`);
    }
    for (const m of matches) {
      let meta = {};
      try { meta = JSON.parse(m.note || '{}'); } catch { errors.push(`rule ${m.ruleId}: note is not valid JSON`); }
      const firstLine = (m.lines || '').split('\n')[0];
      violations.push({
        id: m.ruleId,
        severity: meta.severity || (m.severity === 'error' ? 'high' : 'medium'),
        category: meta.category || 'convention',
        file: m.file,
        line: (m.range?.start?.line ?? 0) + 1,
        fullLine: firstLine,
        text: firstLine.trim().slice(0, 90),
        resolution: meta.resolution || m.message || '',
        engine: 'ast',
      });
    }
    return { available: true, violations, errors };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
```

- [ ] **Step 3: Self-test exercises the ast path — verify green**

Run: `npm run self-test`
Expected: exit 0. Includes either `OK ast-grep canary (...)` (binary present) or `WARN ast-grep unavailable` (binary absent) — both are pass states, no `FAIL`.

- [ ] **Step 4: Verify no temp dir is left behind**

Run: `before=$(ls -d /tmp/slopgate-sg-* 2>/dev/null | wc -l); npm run self-test >/dev/null 2>&1; after=$(ls -d /tmp/slopgate-sg-* 2>/dev/null | wc -l); echo "before=$before after=$after"`
Expected: `after` is not greater than `before` (no new `slopgate-sg-*` dir leaked).

- [ ] **Step 5: Commit**

```bash
git add src/ast-engine.mjs
git commit -m "fix(ast-engine): remove synthesized sgconfig temp dir after scan"
```

---

### Task 4: Extract shared gate-filter helper (DRY cli vs gate)

**Wave:** 1
**Blocks:** Task 5 (both touch `gate.mjs`; Task 5 is Wave 2)
**Blocked by:** —

**Files:**
- Modify: `src/gate.mjs:53-64`
- Modify: `src/cli.mjs:7, 15-23`

**Context:** `cli.mjs:snapshotViolations` re-implements the severity-allow + suppression filter that `gate.mjs:runGate` does at lines 58–64, including the identical "suppressions.json malformed" warning. Extract one exported helper `applyGateFilters(violations, config, mode)` in `gate.mjs` that returns severity-allowed, non-suppressed violations and emits the malformed-suppressions warning once. `runGate` keeps baseline/ratchet filtering (snapshot is intentionally pre-baseline, so it must NOT inherit that).

- [ ] **Step 1: Add `applyGateFilters` to gate.mjs**

In `src/gate.mjs`, add this exported function above `runGate` (after `collectViolations`, before the `runGate` JSDoc):

```js
/**
 * Severity-allow + suppression filter shared by the gate and the baseline snapshot.
 * Emits the malformed-suppressions warning once. Does NOT apply ratchet/baseline.
 * @param {any[]} violations
 * @param {'file'|'staged'} mode  selects the gate.<mode> severity allow-list
 * @returns {any[]}
 */
export function applyGateFilters(violations, config, mode) {
  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);
  return violations
    .filter((v) => allow.has(v.severity))
    .filter((v) => !isSuppressed(sup.entries, v));
}
```

- [ ] **Step 2: Use it inside runGate**

In `src/gate.mjs:runGate`, replace the current filter block (lines 58–64):

```js
  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);

  let violations = collected
    .filter((v) => allow.has(v.severity))
    .filter((v) => !isSuppressed(sup.entries, v));
```

with:

```js
  let violations = applyGateFilters(collected, config, mode);
```

(`loadSuppressions` / `isSuppressed` imports in `gate.mjs` stay — they're now used by `applyGateFilters`.)

- [ ] **Step 3: Use it in cli.mjs:snapshotViolations**

In `src/cli.mjs`, change the gate import (line 4) and rewrite `snapshotViolations` (lines 15–23). Import line:

```js
import { runGate, collectViolations } from './gate.mjs';
```

becomes:

```js
import { runGate, collectViolations, applyGateFilters } from './gate.mjs';
```

and the function:

```js
/** Full-repo commit-tier snapshot, filtered like the gate filters (severity + suppressions). */
function snapshotViolations(config) {
  const { violations, notices } = collectViolations('full', config, 'commit');
  for (const n of notices) process.stderr.write(`⚠ SLOP-GATE: ${n}\n`);
  const allow = new Set(config.gate.staged ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);
  return violations.filter((v) => allow.has(v.severity)).filter((v) => !isSuppressed(sup.entries, v));
}
```

becomes:

```js
/** Full-repo commit-tier snapshot, filtered like the gate filters (severity + suppressions). */
function snapshotViolations(config) {
  const { violations, notices } = collectViolations('full', config, 'commit');
  for (const n of notices) process.stderr.write(`⚠ SLOP-GATE: ${n}\n`);
  return applyGateFilters(violations, config, 'staged');
}
```

- [ ] **Step 4: Drop now-unused cli.mjs imports**

`loadSuppressions` and `isSuppressed` are no longer referenced in `src/cli.mjs`. Change line 7:

```js
import { loadSuppressions, isSuppressed } from './suppressions.mjs';
```

— delete this line entirely. (Verify with `grep -n 'loadSuppressions\|isSuppressed' src/cli.mjs` → no remaining matches after the import is gone.)

- [ ] **Step 5: Run the gate + e2e tests (cover both callers)**

Run: `node src/gate.tier.test.mjs && node src/gate.e2e.test.mjs && node src/ratchet.test.mjs`
Expected: each prints `PASS:` lines, exits 0, no `FAIL`. (The e2e drives `baseline` through the bin → exercises `snapshotViolations`.)

- [ ] **Step 6: Commit**

```bash
git add src/gate.mjs src/cli.mjs
git commit -m "refactor: extract applyGateFilters, dedupe gate vs snapshot filtering"
```

---

### Task 5: Single-pass regex engine + kill the `/g` statefulness bug

**Wave:** 2
**Blocks:** —
**Blocked by:** Task 4 (shares `gate.mjs`)

**Files:**
- Modify: `src/regex-engine.mjs` (full restructure)
- Create: `src/regex-engine.test.mjs`
- Modify: `src/gate.mjs:22` and `src/gate.mjs:2` (import)
- Modify: `src/selftest.mjs:1-5, 17`

**Context — three defects, one restructure:**
1. **`/g` statefulness (bug):** `re.test(line)` in a loop with a `g`-flagged regex advances `lastIndex`, matching only alternate lines (`/x/g`.test → `true,false,true`). Fix: strip `g`/`y` when compiling for line scanning.
2. **Double enumeration:** `gate.collectViolations` computes `files` then `runPatternScan` calls `listSourceFiles` again. Fix: pass `files` in.
3. **Per-pattern re-reads + two-pass expand:** `searchPattern` reads every file once *per pattern*; then `collectRegexViolations` re-reads every file *and re-runs the regex* to recover line numbers it already had. Fix: read each file once, test all patterns, expand recorded hits — merge the two exported functions into one `scanRegex(config, files, { fileMode })`.

`gate.mjs` is the only caller of `runPatternScan`/`collectRegexViolations` (verify: `grep -rn 'runPatternScan\|collectRegexViolations' src` → only `gate.mjs` + `regex-engine.mjs`), so merging them is safe.

- [ ] **Step 1: Write the failing g-flag test**

Create `src/regex-engine.test.mjs`:

```js
// src/regex-engine.test.mjs — locks the single-pass scanner + /g statefulness fix
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { scanRegex, compileLineRegex } from './regex-engine.mjs';

let failed = 0;
const ok = (c, m) => { if (c) console.error(`PASS: ${m}`); else { console.error(`FAIL: ${m}`); failed++; } };

// compileLineRegex strips stateful flags so .test() is repeatable on the same input
const re = compileLineRegex('x', 'gi');
ok(re.test('x') && re.test('x') && re.test('x'), 'compileLineRegex: g/y stripped, .test repeatable');
ok(re.ignoreCase === true, 'compileLineRegex: non-stateful flags (i) preserved');

// scanRegex: a g-flagged rule must hit EVERY matching line, not every other one
const dir = mkdtempSync(join(tmpdir(), 'slopgate-re-'));
try {
  mkdirSync(join(dir, 'src'), { recursive: true });
  writeFileSync(join(dir, 'src/a.ts'), 'BAD\nBAD\nBAD\nok\nBAD\n');
  const config = {
    repoRoot: dir,
    patterns: [{ id: 'bad', severity: 'high', category: 'x', resolution: 'fix', pattern: 'BAD', flags: 'g' }],
  };
  const v = scanRegex(config, ['src/a.ts'], { fileMode: false });
  ok(v.length === 4, `g-flag rule hits all 4 BAD lines (got ${v.length})`);
  ok(v.every((x) => x.engine === 'regex' && x.id === 'bad'), 'violations carry engine=regex + id');
  ok(v[0].line === 1 && v[1].line === 2 && v[2].line === 3 && v[3].line === 5, 'line numbers correct, ok line skipped');
} finally { rmSync(dir, { recursive: true, force: true }); }

process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run it — must fail (module has no `scanRegex`/`compileLineRegex` yet)**

Run: `node src/regex-engine.test.mjs`
Expected: FAIL — throws `SyntaxError`/`does not provide an export named 'scanRegex'` (or `compileLineRegex`).

- [ ] **Step 3: Rewrite regex-engine.mjs as a single-pass scanner**

Replace the **entire** contents of `src/regex-engine.mjs` with:

```js
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { lineHash } from './suppressions.mjs';

// g/y make RegExp.prototype.test stateful (lastIndex advances) → alternate-line
// matches when scanning line-by-line. Strip them for line scanning.
const STATEFUL_FLAGS = /[gy]/g;

/** Compile a rule's regex for line-by-line scanning (never stateful). */
export function compileLineRegex(pattern, flags) {
  const safe = (flags || '').replace(STATEFUL_FLAGS, '');
  return new RegExp(pattern, safe || undefined);
}

function pathMatchesGlobs(filePath, globs) {
  if (!globs?.length) return false;
  return globs.some((g) => {
    const norm = g
      .replace(/[.+^${}()|[\]\\]/g, '\\$&')
      .replace(/\*\*\//g, '\x00')
      .replace(/\*\*/g, '\x01')
      .replace(/\*/g, '[^/]*')
      .replace(/\x00/g, '(?:.*/)?')
      .replace(/\x01/g, '.*');
    return new RegExp('^' + norm + '$').test(filePath);
  });
}

/**
 * Single-pass regex scan: read each file once, test every pattern against every line,
 * apply per-pattern minFiles thresholds, expand surviving hits into violations.
 * @param {import('./config.mjs').ResolvedConfig} config
 * @param {string[]} files repo-relative source paths (already enumerated by the caller)
 * @param {{ fileMode?: boolean }} [opts] fileMode → drop cross-file (minFiles>1) rules
 * @returns {{ id:string, severity:string, category:string, file:string, line:number, lineHash:string, text:string, resolution:string, engine:'regex' }[]}
 */
export function scanRegex(config, files, { fileMode = false } = {}) {
  const compiled = [];
  for (const p of config.patterns) {
    if (fileMode && (p.minFiles ?? 1) > 1) continue; // cross-file thresholds meaningless on one file
    try { compiled.push({ p, re: compileLineRegex(p.pattern, p.flags) }); }
    catch { /* unparseable pattern: skip, mirrors prior swallow */ }
  }

  // pass 1: one read per file; record hits per pattern as file -> [{line, text}]
  const hits = new Map(); // patternId -> Map(file -> [{line, text}])
  for (const file of files) {
    let lines;
    try { lines = readFileSync(join(config.repoRoot, file), 'utf8').split('\n'); }
    catch { continue; }
    for (const { p, re } of compiled) {
      if (pathMatchesGlobs(file, p.excludeGlobs)) continue;
      let perFile = null;
      for (let i = 0; i < lines.length; i++) {
        if (re.test(lines[i])) {
          (perFile ??= []).push({ line: i + 1, text: lines[i] });
        }
      }
      if (perFile) {
        let byFile = hits.get(p.id);
        if (!byFile) { byFile = new Map(); hits.set(p.id, byFile); }
        byFile.set(file, perFile);
      }
    }
  }

  // pass 2: minFiles threshold + expand to violations (no re-read, no re-test)
  const violations = [];
  for (const { p } of compiled) {
    const byFile = hits.get(p.id);
    if (!byFile || byFile.size < (p.minFiles ?? 1)) continue;
    for (const file of [...byFile.keys()].sort()) {
      for (const { line, text } of byFile.get(file)) {
        violations.push({
          id: p.id, severity: p.severity, category: p.category, file, line,
          lineHash: lineHash(text),
          text: text.trim().slice(0, 90),
          resolution: p.resolution, engine: 'regex',
        });
      }
    }
  }
  return violations;
}
```

(Note: `compiled` may list the same `p.id` twice only if config had duplicate ids — but `config.mjs` dedupes by id upstream, so `hits.get(p.id)` is unambiguous. The pass-2 loop over `compiled` therefore visits each surviving id once.)

- [ ] **Step 4: Run the new test — must pass**

Run: `node src/regex-engine.test.mjs`
Expected: all `PASS:` lines, exit 0.

- [ ] **Step 5: Switch gate.mjs to scanRegex with the pre-computed file list**

In `src/gate.mjs`, change the import (line 2):

```js
import { runPatternScan, collectRegexViolations } from './regex-engine.mjs';
```

to:

```js
import { scanRegex } from './regex-engine.mjs';
```

and in `collectViolations`, replace line 22:

```js
  const violations = collectRegexViolations(config, runPatternScan(config, opts));
```

with (reuse the `files` already computed on line 18; `mode === 'file'` selects fileMode):

```js
  const violations = scanRegex(config, files, { fileMode: mode === 'file' });
```

- [ ] **Step 6: Point selftest.mjs at compileLineRegex**

In `src/selftest.mjs`, add the import and use it so canary compilation matches engine compilation exactly. Add to the imports (after line 4 `runAstGrepScan` import is fine, or alongside line 1 group):

```js
import { compileLineRegex } from './regex-engine.mjs';
```

and replace line 17:

```js
    try { re = new RegExp(p.pattern, p.flags || undefined); } catch (e) { console.error(`FAIL ${p.id}: regex invalid: ${e}`); failed++; continue; }
```

with:

```js
    try { re = compileLineRegex(p.pattern, p.flags); } catch (e) { console.error(`FAIL ${p.id}: regex invalid: ${e}`); failed++; continue; }
```

- [ ] **Step 7: Full regression — every test file + self-test**

Run: `for f in $(find src -name '*.test.mjs'); do node "$f" >/dev/null 2>&1 && echo "ok $f" || echo "FAIL $f"; done; npm run self-test && echo "selftest-ok"`
Expected: every line `ok src/...`, then self-test `OK` lines, then `selftest-ok`. No `FAIL`.

- [ ] **Step 8: Behavior-parity smoke check on this repo**

Run: `node bin/slop-gate --self-test --config rules/baseline/selftest.config.mjs; echo "exit=$?"`
Expected: `exit=0` (self-test config has canaries that fire; identical OK lines to baseline run before the task).

- [ ] **Step 9: Commit**

```bash
git add src/regex-engine.mjs src/regex-engine.test.mjs src/gate.mjs src/selftest.mjs
git commit -m "perf(regex): single-pass scan, fix /g statefulness, drop double enumeration"
```

---

## Self-Review

**1. Spec coverage:**
- C1 (`/g` bug) → Task 5 (`compileLineRegex` + test). ✓
- C2 (double enumeration) → Task 5 Step 5 (pass `files` in). ✓
- C3 (two-pass/per-pattern re-read) → Task 5 Step 3 (`scanRegex` single pass). ✓
- D1 (gate-filter DRY) → Task 4. ✓
- D2 (`ENGINE_ROOT` dup) → Task 2. ✓
- D3 (config dedupe) → Task 1. ✓
- Out-of-scope temp-dir extra → Task 3. ✓
- init.mjs split → intentionally omitted (spec: rejected). ✓

**2. Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to" — every code step has full literal code. ✓

**3. Type/name consistency:** `scanRegex(config, files, { fileMode })`, `compileLineRegex(pattern, flags)`, `applyGateFilters(violations, config, mode)`, `ENGINE_ROOT` — each defined once and referenced with the same signature everywhere (Task 5 defines `scanRegex`; gate.mjs Step 5 calls it with matching args; Task 4 defines `applyGateFilters`, gate+cli call it with `mode`). ✓

**4. Wave plan check:** Every task has Wave/Blocks/Blocked-by. Wave 1 = Tasks 1–4; their file sets — `{config.mjs}`, `{install-hooks.mjs, init.mjs}`, `{ast-engine.mjs}`, `{gate.mjs, cli.mjs}` — are pairwise disjoint. ✓ Task 5 touches `gate.mjs` (also in Task 4) → correctly placed in Wave 2, `Blocked by: Task 4`. ✓ No same-wave file overlap.
