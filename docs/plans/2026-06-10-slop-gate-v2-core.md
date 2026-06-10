# Slop-Gate v2 Core Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use /sdd (ship unavailable in this env) or /executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two-tier gate: fast tier (post-edit, unchanged) + commit tier adding six checker adapters (tsc, knip, jscpd, depcruise, type-coverage, diff-shape), a fingerprint ratchet baseline (only NEW violations fail), and a native git pre-commit installer.

**Architecture:** Checkers are adapter modules (`detect`/`run`) emitting the existing violation shape; gate runs them in commit tier only, then filters via `.slop-gate/baseline.json` fingerprints. Parsers are pure functions verified by recorded-output fixtures in self-test. Plan 2 (rule packs) and Plan 3 (audit command) follow separately — spec §3.5/§3.9.

**Tech Stack:** Node 20+ ESM (`.mjs`), no runtime deps. External tools resolved from target repo's `node_modules/.bin` only. Tests = plain node scripts with PASS/FAIL lines + exit code (existing `src/init.test.mjs` convention).

**Spec:** `docs/specs/2026-06-10-slop-gate-v2-design.md`

**Plan deviations from spec (agreed):** config keys for checkers use quoted kebab-case matching checker ids (`'type-coverage'`, `'diff-shape'`) — no camelCase aliasing. `init` prints a "run slop-gate baseline" next-step instead of auto-generating the baseline (avoids snapshotting mid-setup).

---

## Wave Plan

| Wave | Tasks | Files touched | Safe to parallelize? |
|------|-------|---------------|----------------------|
| 1 | T1 ratchet, T2 checkers/shared, T3 install-hooks, T4 config | src/ratchet.mjs(+test), src/checkers/shared.mjs(+test), src/install-hooks.mjs(+test), src/config.mjs | ✅ no overlap |
| 2 | T5 tsc, T6 knip, T7 jscpd, T8 depcruise, T9 type-coverage, T10 diff-shape | one adapter file + own fixture files each | ✅ no overlap |
| 3 | T11 registry+gate+report, T12 selftest fixtures stage, T13 init updates | src/checkers/index.mjs+src/gate.mjs+src/report.mjs(+test) / src/selftest.mjs / src/init.mjs | ✅ no overlap |
| 4 | T14 CLI | src/cli.mjs | single task |
| 5 | T15 e2e smoke | src/gate.e2e.test.mjs | single task |

---

### Task 1: Ratchet baseline module

**Wave:** 1
**Blocks:** T11, T14
**Blocked by:** —

**Files:**
- Create: `src/ratchet.mjs`
- Test: `src/ratchet.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/ratchet.test.mjs
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fingerprintViolation, loadBaseline, filterNew, writeBaseline } from './ratchet.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const v1 = { engine: 'checker:tsc', id: 'tsc-TS2322', file: 'src/a.ts', text: "Type 'string' is not assignable at 12", fullLine: 'const x: number = s;' };
const v2 = { ...v1, file: 'src/b.ts' };

// fingerprint: stable, digit-normalized, line-text-sensitive
assert('same violation → same fp', fingerprintViolation(v1) === fingerprintViolation({ ...v1 }));
assert('digits in message normalized', fingerprintViolation(v1) === fingerprintViolation({ ...v1, text: "Type 'string' is not assignable at 99" }));
assert('different file → different fp', fingerprintViolation(v1) !== fingerprintViolation(v2));
assert('different line text → different fp', fingerprintViolation(v1) !== fingerprintViolation({ ...v1, fullLine: 'const y: number = s;' }));
assert('fp is 16 hex chars', /^[0-9a-f]{16}$/.test(fingerprintViolation(v1)));

const dir = mkdtempSync(join(tmpdir(), 'slopgate-ratchet-'));
const blPath = join(dir, 'baseline.json');

// missing baseline
const missing = loadBaseline(blPath);
assert('missing → empty entries + missing flag', missing.missing === true && Object.keys(missing.entries).length === 0 && missing.error === null);

// write + load round-trip
const n = writeBaseline(blPath, [v1, v2], '2026-06-10T00:00:00Z');
assert('writeBaseline returns entry count', n === 2);
const loaded = loadBaseline(blPath);
assert('round-trip 2 entries', Object.keys(loaded.entries).length === 2 && loaded.missing === false);
assert('entry carries ruleId+file', loaded.entries[fingerprintViolation(v1)].ruleId === 'tsc-TS2322' && loaded.entries[fingerprintViolation(v1)].file === 'src/a.ts');

// filterNew
const v3 = { ...v1, id: 'tsc-TS7006', text: 'Parameter implicitly any' };
const { fresh, baselinedCount } = filterNew([v1, v2, v3], loaded.entries);
assert('baselined dropped', baselinedCount === 2);
assert('new survives', fresh.length === 1 && fresh[0].id === 'tsc-TS7006');

// malformed
writeFileSync(blPath, '{ not json');
const bad = loadBaseline(blPath);
assert('malformed → empty + error', bad.error !== null && Object.keys(bad.entries).length === 0 && bad.missing === false);
writeFileSync(blPath, JSON.stringify({ version: 1, entries: [] }));
assert('entries-as-array → error', loadBaseline(blPath).error !== null);

rmSync(dir, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/ratchet.test.mjs`
Expected: FAIL — `Cannot find module './ratchet.mjs'`

- [ ] **Step 3: Implement**

```js
// src/ratchet.mjs
/**
 * Ratchet baseline: snapshot existing violations; gate fails only on NEW ones.
 * Fingerprint = sha256(engine|id|file|digit-normalized message|trimmed line text), 16 hex.
 * Line numbers excluded → survives unrelated line shifts. Identical (file,rule,line-text)
 * duplicates collapse to one fingerprint — acceptable for "did something NEW appear".
 * Malformed baseline → treated as empty with error surfaced (fail toward blocking).
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';

export function fingerprintViolation(v) {
  const msg = String(v.text ?? '').replace(/\d+/g, '#');
  const line = String(v.fullLine ?? '').trim();
  return createHash('sha256')
    .update([v.engine ?? '', v.id, v.file, msg, line].join('|'))
    .digest('hex').slice(0, 16);
}

export function loadBaseline(path) {
  if (!path || !existsSync(path)) return { entries: {}, missing: true, error: null };
  try {
    const j = JSON.parse(readFileSync(path, 'utf8'));
    if (!j.entries || typeof j.entries !== 'object' || Array.isArray(j.entries)) {
      throw new Error('"entries" is not an object');
    }
    return { entries: j.entries, missing: false, error: null };
  } catch (err) {
    return { entries: {}, missing: false, error: String(err) };
  }
}

export function filterNew(violations, entries) {
  const fresh = [];
  let baselinedCount = 0;
  for (const v of violations) {
    if (entries[fingerprintViolation(v)]) baselinedCount++;
    else fresh.push(v);
  }
  return { fresh, baselinedCount };
}

export function writeBaselineRaw(path, entries, generated) {
  writeFileSync(path, `${JSON.stringify({ version: 1, generated, entries }, null, 2)}\n`);
  return Object.keys(entries).length;
}

export function writeBaseline(path, violations, generated) {
  const entries = {};
  for (const v of violations) entries[fingerprintViolation(v)] = { ruleId: v.id, file: v.file };
  return writeBaselineRaw(path, entries, generated);
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node src/ratchet.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 5: Commit**

```bash
git add src/ratchet.mjs src/ratchet.test.mjs
git commit -m "feat(ratchet): fingerprint baseline — load/filterNew/write"
```

---

### Task 2: Checker shared helpers

**Wave:** 1
**Blocks:** T5–T9
**Blocked by:** —

**Files:**
- Create: `src/checkers/shared.mjs`
- Test: `src/checkers/shared.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/checkers/shared.test.mjs
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { localBin, runTool, sourceLine } from './shared.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const root = mkdtempSync(join(tmpdir(), 'slopgate-shared-'));

// localBin
assert('missing bin → null', localBin(root, 'tsc') === null);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '#!/bin/sh\necho ok\n');
chmodSync(join(root, 'node_modules/.bin/tsc'), 0o755);
assert('present bin → path', localBin(root, 'tsc') === join(root, 'node_modules/.bin/tsc'));

// runTool
const ok = runTool('node', ['-e', 'console.log("hi"); process.exit(3)'], { cwd: root, timeout: 10_000 });
assert('captures stdout', ok.ok === true && ok.stdout.trim() === 'hi');
assert('captures status', ok.status === 3);
const bad = runTool('/nonexistent-binary-xyz', [], { cwd: root, timeout: 5_000 });
assert('spawn failure → ok:false + error', bad.ok === false && bad.error.length > 0);
const slow = runTool('node', ['-e', 'setTimeout(()=>{}, 60000)'], { cwd: root, timeout: 300 });
assert('timeout → ok:false', slow.ok === false);

// sourceLine
writeFileSync(join(root, 'f.ts'), 'line one\nline two\n');
assert('reads 1-based line', sourceLine(root, 'f.ts', 2) === 'line two');
assert('out of range → empty', sourceLine(root, 'f.ts', 99) === '');
assert('missing file → empty', sourceLine(root, 'nope.ts', 1) === '');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/checkers/shared.test.mjs`
Expected: FAIL — `Cannot find module './shared.mjs'`

- [ ] **Step 3: Implement**

```js
// src/checkers/shared.mjs
/** Shared checker plumbing: local-bin resolution (no global/npx — deterministic versions),
 *  spawn wrapper (timeout → ok:false, never throws), source-line lookup for lineHash. */
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

export function localBin(repoRoot, name) {
  const p = join(repoRoot, 'node_modules/.bin', name);
  return existsSync(p) ? p : null;
}

export function runTool(bin, args, { cwd, timeout }) {
  const res = spawnSync(bin, args, { encoding: 'utf8', cwd, timeout, maxBuffer: 64 * 1024 * 1024 });
  if (res.error) return { ok: false, error: String(res.error), stdout: res.stdout ?? '', stderr: res.stderr ?? '', status: null };
  return { ok: true, error: null, stdout: res.stdout ?? '', stderr: res.stderr ?? '', status: res.status };
}

export function sourceLine(repoRoot, file, line) {
  try { return readFileSync(join(repoRoot, file), 'utf8').split('\n')[line - 1] ?? ''; }
  catch { return ''; }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node src/checkers/shared.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 5: Commit**

```bash
git add src/checkers/shared.mjs src/checkers/shared.test.mjs
git commit -m "feat(checkers): shared helpers — localBin/runTool/sourceLine"
```

---

### Task 3: Native pre-commit installer

**Wave:** 1
**Blocks:** T13, T14
**Blocked by:** —

**Files:**
- Create: `src/install-hooks.mjs`
- Test: `src/install-hooks.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/install-hooks.test.mjs
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, statSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { installPreCommitHook, renderHookContent, MARKER_BEGIN } from './install-hooks.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

// pure rendering
const fresh = renderHookContent(null);
assert('fresh: shebang + marker + invocation', fresh.action === 'created'
  && fresh.content.startsWith('#!/usr/bin/env bash')
  && fresh.content.includes(MARKER_BEGIN)
  && fresh.content.includes('--staged --config'));

const again = renderHookContent(fresh.content);
assert('re-render same content → unchanged', again.action === 'unchanged' && again.content === fresh.content);

const foreignTail = '#!/bin/sh\necho lint\n';
const appended = renderHookContent(foreignTail);
assert('foreign no-exec → appended at end', appended.action === 'appended'
  && appended.content.startsWith('#!/bin/sh\necho lint')
  && appended.content.includes(MARKER_BEGIN));

const foreignExec = '#!/bin/sh\necho pre\nexec husky run\n';
const beforeExec = renderHookContent(foreignExec);
const lines = beforeExec.content.split('\n');
assert('foreign exec → our block before exec', beforeExec.action === 'appended'
  && lines.indexOf(MARKER_BEGIN) < lines.findIndex((l) => /^\s*exec\s/.test(l)));

// real git repo install
const repo = mkdtempSync(join(tmpdir(), 'slopgate-hookinstall-'));
execSync('git init -q', { cwd: repo });
const r1 = installPreCommitHook(repo);
const hookPath = join(repo, '.git/hooks/pre-commit');
assert('install creates hook', r1.action === 'created' && existsSync(hookPath));
assert('hook is executable', (statSync(hookPath).mode & 0o100) !== 0);
const r2 = installPreCommitHook(repo);
assert('idempotent', r2.action === 'unchanged');

// core.hooksPath respected
const repo2 = mkdtempSync(join(tmpdir(), 'slopgate-hookpath-'));
execSync('git init -q', { cwd: repo2 });
mkdirSync(join(repo2, 'custom-hooks'));
execSync('git config core.hooksPath custom-hooks', { cwd: repo2 });
const r3 = installPreCommitHook(repo2);
assert('core.hooksPath used', r3.action === 'created' && existsSync(join(repo2, 'custom-hooks/pre-commit')));

rmSync(repo, { recursive: true, force: true });
rmSync(repo2, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/install-hooks.test.mjs`
Expected: FAIL — `Cannot find module './install-hooks.mjs'`

- [ ] **Step 3: Implement**

```js
// src/install-hooks.mjs
/**
 * Native git pre-commit installer. Marker-delimited block: idempotent rewrite of our
 * block, foreign hook content always preserved (block inserted before first `exec`,
 * else appended). Engine path embedded absolute — single-machine personal tool.
 */
import { execSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync, chmodSync, mkdirSync } from 'node:fs';
import { join, isAbsolute } from 'node:path';

const ENGINE_ROOT = '/home/user/Projects/slop-gate';
export const MARKER_BEGIN = '# slop-gate-hook v1 BEGIN';
export const MARKER_END = '# slop-gate-hook v1 END';

function hookBlock() {
  return [
    MARKER_BEGIN,
    'SLOPGATE_ROOT=$(git rev-parse --show-toplevel 2>/dev/null)',
    'if [ -n "$SLOPGATE_ROOT" ] && [ -f "$SLOPGATE_ROOT/.slop-gate/config.mjs" ]; then',
    `  node ${ENGINE_ROOT}/bin/slop-gate --staged --config "$SLOPGATE_ROOT/.slop-gate/config.mjs" || exit 1`,
    'fi',
    MARKER_END,
  ].join('\n');
}

export function renderHookContent(existing) {
  const block = hookBlock();
  if (!existing) return { content: `#!/usr/bin/env bash\n${block}\n`, action: 'created' };
  if (existing.includes(MARKER_BEGIN)) {
    const start = existing.indexOf(MARKER_BEGIN);
    const end = existing.indexOf(MARKER_END) + MARKER_END.length;
    const content = existing.slice(0, start) + block + existing.slice(end);
    return { content, action: content === existing ? 'unchanged' : 'updated' };
  }
  const lines = existing.split('\n');
  const execIdx = lines.findIndex((l) => /^\s*exec\s/.test(l));
  if (execIdx === -1) {
    return { content: `${existing.replace(/\n*$/, '\n')}${block}\n`, action: 'appended' };
  }
  lines.splice(execIdx, 0, block);
  return { content: lines.join('\n'), action: 'appended' };
}

export function resolveHooksDir(repoRoot) {
  let hooksPath = '';
  try { hooksPath = execSync('git config core.hooksPath', { cwd: repoRoot, encoding: 'utf8' }).trim(); } catch { /* unset */ }
  if (hooksPath) return isAbsolute(hooksPath) ? hooksPath : join(repoRoot, hooksPath);
  const gitDir = execSync('git rev-parse --git-dir', { cwd: repoRoot, encoding: 'utf8' }).trim();
  return join(isAbsolute(gitDir) ? gitDir : join(repoRoot, gitDir), 'hooks');
}

/** @returns {{ action: 'created'|'updated'|'appended'|'unchanged', path: string }} */
export function installPreCommitHook(repoRoot) {
  const hooksDir = resolveHooksDir(repoRoot);
  mkdirSync(hooksDir, { recursive: true });
  const path = join(hooksDir, 'pre-commit');
  const existing = existsSync(path) ? readFileSync(path, 'utf8') : null;
  const { content, action } = renderHookContent(existing);
  if (action !== 'unchanged') writeFileSync(path, content);
  chmodSync(path, 0o755);
  return { action, path };
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node src/install-hooks.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 5: Commit**

```bash
git add src/install-hooks.mjs src/install-hooks.test.mjs
git commit -m "feat(hooks): native pre-commit installer — marker block, foreign-hook safe"
```

---

### Task 4: Config — checkers, astDisable, baselinePath

**Wave:** 1
**Blocks:** T11, T14
**Blocked by:** —

**Files:**
- Modify: `src/config.mjs` (inside `resolveConfig`, before the `return`; and the returned object)
- Test: `src/config.checkers.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/config.checkers.test.mjs
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { resolveConfig } from './config.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const repo = mkdtempSync(join(tmpdir(), 'slopgate-cfg-'));
execSync('git init -q', { cwd: repo });
mkdirSync(join(repo, '.slop-gate'), { recursive: true });
mkdirSync(join(repo, 'src'), { recursive: true });
writeFileSync(join(repo, '.slop-gate/config.mjs'), `export default {
  roots: ['src'],
  astDisable: ['console-debug-left'],
  checkers: {
    tsc: true,
    knip: false,
    jscpd: { minTokens: 70 },
  },
};\n`);

const cfg = await resolveConfig(join(repo, '.slop-gate/config.mjs'));
assert('true normalizes to {}', JSON.stringify(cfg.checkers.tsc) === '{}');
assert('false drops the checker', !('knip' in cfg.checkers));
assert('object passes through', cfg.checkers.jscpd.minTokens === 70);
assert('absent checker absent', !('depcruise' in cfg.checkers));
assert('astDisable is a Set', cfg.astDisable instanceof Set && cfg.astDisable.has('console-debug-left'));
assert('baselinePath under configDir', cfg.baselinePath === join(repo, '.slop-gate/baseline.json'));

// defaults when keys absent (separate config file — ESM import cache keys by path)
writeFileSync(join(repo, '.slop-gate/config2.mjs'), 'export default { roots: ["src"] };\n');
const cfg2 = await resolveConfig(join(repo, '.slop-gate/config2.mjs'));
assert('no checkers key → empty object', Object.keys(cfg2.checkers).length === 0);
assert('no astDisable → empty Set', cfg2.astDisable instanceof Set && cfg2.astDisable.size === 0);

rmSync(repo, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/config.checkers.test.mjs`
Expected: FAIL lines (cfg.checkers undefined)

- [ ] **Step 3: Implement**

In `src/config.mjs`, after the `astRuleDirs` block and before `const rootsRel = ...`, add:

```js
  // commit-tier checkers: absent/false => disabled; true => {}; object => options
  const checkers = {};
  for (const [name, v] of Object.entries(raw.checkers ?? {})) {
    if (v === false || v == null) continue;
    checkers[name] = v === true ? {} : v;
  }
```

In the returned object, add three keys (after `astRuleDirs,`):

```js
    checkers,
    astDisable: new Set(raw.astDisable ?? []),
    baselinePath: join(configDir, 'baseline.json'),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node src/config.checkers.test.mjs`
Expected: all PASS, exit 0. Also run `npm run self-test` — must still pass (no regression).

- [ ] **Step 5: Commit**

```bash
git add src/config.mjs src/config.checkers.test.mjs
git commit -m "feat(config): checkers map, astDisable set, baselinePath"
```

---

### Task 5: tsc checker

**Wave:** 2
**Blocks:** T11, T12
**Blocked by:** T2

**Files:**
- Create: `src/checkers/tsc.mjs`
- Create: `rules/baseline/fixtures/checker-outputs/tsc.txt`
- Create: `rules/baseline/fixtures/checker-outputs/tsc.expected.json`
- Test: `src/checkers/tsc.test.mjs`

- [ ] **Step 1: Record the parser fixture**

`rules/baseline/fixtures/checker-outputs/tsc.txt`:

```text
src/a.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.
src/b.tsx(3,1): error TS2304: Cannot find name 'foo'.
src/long.ts(7,9): error TS2345: Argument of type '{ a: string; }' is not assignable to parameter of type 'Opts'.
  Property 'b' is missing in type '{ a: string; }' but required in type 'Opts'.
```

`rules/baseline/fixtures/checker-outputs/tsc.expected.json`:

```json
[
  { "file": "src/a.ts", "line": 12, "code": "TS2322", "message": "Type 'string' is not assignable to type 'number'." },
  { "file": "src/b.tsx", "line": 3, "code": "TS2304", "message": "Cannot find name 'foo'." },
  { "file": "src/long.ts", "line": 7, "code": "TS2345", "message": "Argument of type '{ a: string; }' is not assignable to parameter of type 'Opts'. Property 'b' is missing in type '{ a: string; }' but required in type 'Opts'." }
]
```

- [ ] **Step 2: Write the failing test**

```js
// src/checkers/tsc.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import tsc, { parseTscOutput } from './tsc.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseTscOutput(readFileSync(join(fixDir, 'tsc.txt'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'tsc.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));
assert('empty output → no errors', parseTscOutput('').length === 0);

// detect
const root = mkdtempSync(join(tmpdir(), 'slopgate-tsc-'));
assert('no tsconfig → unavailable', tsc.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'tsconfig.json'), '{}');
assert('no local tsc → unavailable', tsc.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '');
assert('tsconfig + bin → available', tsc.detect({ repoRoot: root }, {}).available === true);
assert('custom tsconfig honored', tsc.detect({ repoRoot: root }, { tsconfig: 'tsconfig.app.json' }).available === false);
assert('id', tsc.id === 'tsc');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 3: Run test to verify it fails**

Run: `node src/checkers/tsc.test.mjs`
Expected: FAIL — `Cannot find module './tsc.mjs'`

- [ ] **Step 4: Implement**

```js
// src/checkers/tsc.mjs
/** tsc --noEmit adapter. Always full-project: a staged change can break a non-staged
 *  file and that MUST fail; pre-existing errors are absorbed by the ratchet baseline. */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseTscOutput(stdout) {
  const errors = [];
  for (const raw of stdout.split('\n')) {
    const m = /^(.+?)\((\d+),(\d+)\): error (TS\d+): (.*)$/.exec(raw);
    if (m) {
      errors.push({ file: m[1].replace(/\\/g, '/'), line: Number(m[2]), code: m[4], message: m[5] });
    } else if (errors.length && /^\s+\S/.test(raw)) {
      errors[errors.length - 1].message += ` ${raw.trim()}`;
    }
  }
  return errors;
}

export default {
  id: 'tsc',
  detect(config, cfg) {
    const tsconfig = join(config.repoRoot, cfg.tsconfig ?? 'tsconfig.json');
    if (!existsSync(tsconfig)) return { available: false, reason: `no ${cfg.tsconfig ?? 'tsconfig.json'}` };
    if (!localBin(config.repoRoot, 'tsc')) return { available: false, reason: 'no local tsc binary' };
    return { available: true };
  },
  run(config, cfg) {
    const tsconfig = join(config.repoRoot, cfg.tsconfig ?? 'tsconfig.json');
    const res = runTool(localBin(config.repoRoot, 'tsc'), ['--noEmit', '--pretty', 'false', '-p', tsconfig], {
      cwd: config.repoRoot, timeout: (cfg.timeout ?? 120) * 1000,
    });
    if (!res.ok) return { violations: [], errors: [`tsc failed: ${res.error}`] };
    const violations = parseTscOutput(res.stdout).map((e) => ({
      id: `tsc-${e.code}`, severity: 'high', category: 'types',
      file: e.file, line: e.line,
      fullLine: sourceLine(config.repoRoot, e.file, e.line),
      text: e.message.trim().slice(0, 90),
      resolution: 'Fix the type error — do not suppress.',
    }));
    return { violations, errors: [] };
  },
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `node src/checkers/tsc.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 6: Commit**

```bash
git add src/checkers/tsc.mjs src/checkers/tsc.test.mjs rules/baseline/fixtures/checker-outputs/tsc.txt rules/baseline/fixtures/checker-outputs/tsc.expected.json
git commit -m "feat(checkers): tsc adapter + parser fixture"
```

---

### Task 6: knip checker

**Wave:** 2
**Blocks:** T11, T12
**Blocked by:** T2

**Files:**
- Create: `src/checkers/knip.mjs`
- Create: `rules/baseline/fixtures/checker-outputs/knip.json`
- Create: `rules/baseline/fixtures/checker-outputs/knip.expected.json`
- Test: `src/checkers/knip.test.mjs`

- [ ] **Step 1: Record the parser fixture**

`rules/baseline/fixtures/checker-outputs/knip.json` (shape of `knip --reporter json`):

```json
{
  "files": ["src/orphan.ts"],
  "issues": [
    {
      "file": "src/util.ts",
      "dependencies": [],
      "devDependencies": [],
      "optionalPeerDependencies": [],
      "unlisted": [],
      "binaries": [],
      "unresolved": [],
      "exports": [{ "name": "unusedHelper", "line": 14, "col": 14, "pos": 300 }],
      "types": [{ "name": "UnusedType", "line": 2, "col": 13, "pos": 30 }],
      "enumMembers": {},
      "duplicates": []
    },
    {
      "file": "package.json",
      "dependencies": [{ "name": "left-pad", "line": 12, "col": 6, "pos": 200 }],
      "devDependencies": [],
      "optionalPeerDependencies": [],
      "unlisted": [],
      "binaries": [],
      "unresolved": [],
      "exports": [],
      "types": [],
      "enumMembers": {},
      "duplicates": []
    }
  ]
}
```

`rules/baseline/fixtures/checker-outputs/knip.expected.json`:

```json
[
  { "type": "files", "file": "src/orphan.ts", "line": 1, "name": "src/orphan.ts" },
  { "type": "exports", "file": "src/util.ts", "line": 14, "name": "unusedHelper" },
  { "type": "types", "file": "src/util.ts", "line": 2, "name": "UnusedType" },
  { "type": "dependencies", "file": "package.json", "line": 12, "name": "left-pad" }
]
```

- [ ] **Step 2: Write the failing test**

```js
// src/checkers/knip.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import knip, { parseKnipOutput } from './knip.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseKnipOutput(readFileSync(join(fixDir, 'knip.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'knip.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));
assert('empty report → none', parseKnipOutput('{"files":[],"issues":[]}').length === 0);

const root = mkdtempSync(join(tmpdir(), 'slopgate-knip-'));
writeFileSync(join(root, 'package.json'), '{}');
assert('no bin → unavailable', knip.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/knip'), '');
assert('bin but no knip config → unavailable', knip.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'knip.json'), '{}');
assert('bin + knip.json → available', knip.detect({ repoRoot: root }, {}).available === true);
rmSync(join(root, 'knip.json'));
writeFileSync(join(root, 'package.json'), '{"knip":{}}');
assert('pkg.knip counts as config', knip.detect({ repoRoot: root }, {}).available === true);
assert('id', knip.id === 'knip');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 3: Run test to verify it fails**

Run: `node src/checkers/knip.test.mjs`
Expected: FAIL — `Cannot find module './knip.mjs'`

- [ ] **Step 4: Implement**

```js
// src/checkers/knip.mjs
/** knip adapter — unused files/exports/types/deps. Full-repo by nature (dead code is a
 *  whole-graph property); pre-existing findings are absorbed by the ratchet baseline.
 *  Requires explicit knip config — knip without config is too noisy to gate on. */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool, sourceLine } from './shared.mjs';

const ISSUE_TYPES = ['dependencies', 'devDependencies', 'unlisted', 'exports', 'types', 'duplicates'];

const RESOLUTIONS = {
  files: 'Delete the unused file (or wire it in if it was meant to be used).',
  exports: 'Remove the unused export (or its consumer was deleted by mistake).',
  types: 'Remove the unused exported type.',
  dependencies: 'Uninstall the unused dependency.',
  devDependencies: 'Uninstall the unused devDependency.',
  unlisted: 'Add the dependency to package.json (it is imported but unlisted).',
  duplicates: 'Deduplicate the export.',
};

export function parseKnipOutput(jsonText) {
  const j = JSON.parse(jsonText);
  const out = [];
  for (const f of j.files ?? []) out.push({ type: 'files', file: f, line: 1, name: f });
  for (const issue of j.issues ?? []) {
    for (const type of ISSUE_TYPES) {
      for (const item of issue[type] ?? []) {
        out.push({ type, file: issue.file, line: item.line ?? 1, name: item.name ?? String(item) });
      }
    }
  }
  return out;
}

function hasKnipConfig(repoRoot) {
  if (['knip.json', 'knip.jsonc', 'knip.config.ts', 'knip.config.js'].some((f) => existsSync(join(repoRoot, f)))) return true;
  try { return 'knip' in JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8')); }
  catch { return false; }
}

export default {
  id: 'knip',
  detect(config) {
    if (!localBin(config.repoRoot, 'knip')) return { available: false, reason: 'no local knip binary' };
    if (!hasKnipConfig(config.repoRoot)) return { available: false, reason: 'no knip config' };
    return { available: true };
  },
  run(config, cfg) {
    const res = runTool(localBin(config.repoRoot, 'knip'), ['--reporter', 'json', '--no-exit-code'], {
      cwd: config.repoRoot, timeout: (cfg.timeout ?? 90) * 1000,
    });
    if (!res.ok) return { violations: [], errors: [`knip failed: ${res.error}`] };
    let findings;
    try { findings = parseKnipOutput(res.stdout); }
    catch (e) { return { violations: [], errors: [`knip JSON parse error: ${e}`] }; }
    const violations = findings.map((f) => ({
      id: `knip-${f.type}`, severity: 'high', category: 'dead-code',
      file: f.file, line: f.line,
      fullLine: f.type === 'files' ? '' : sourceLine(config.repoRoot, f.file, f.line),
      text: `unused ${f.type === 'files' ? 'file' : f.type}: ${f.name}`.slice(0, 90),
      resolution: RESOLUTIONS[f.type],
    }));
    return { violations, errors: [] };
  },
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `node src/checkers/knip.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 6: Commit**

```bash
git add src/checkers/knip.mjs src/checkers/knip.test.mjs rules/baseline/fixtures/checker-outputs/knip.json rules/baseline/fixtures/checker-outputs/knip.expected.json
git commit -m "feat(checkers): knip adapter + parser fixture"
```

---

### Task 7: jscpd checker

**Wave:** 2
**Blocks:** T11, T12
**Blocked by:** T2

**Files:**
- Create: `src/checkers/jscpd.mjs`
- Create: `rules/baseline/fixtures/checker-outputs/jscpd.json`
- Create: `rules/baseline/fixtures/checker-outputs/jscpd.expected.json`
- Test: `src/checkers/jscpd.test.mjs`

- [ ] **Step 1: Record the parser fixture**

`rules/baseline/fixtures/checker-outputs/jscpd.json` (shape of jscpd's `jscpd-report.json`):

```json
{
  "duplicates": [
    {
      "format": "typescript",
      "lines": 18,
      "tokens": 120,
      "firstFile": { "name": "src/features/a.ts", "start": 10, "end": 27, "startLoc": { "line": 10, "column": 1 }, "endLoc": { "line": 27, "column": 2 } },
      "secondFile": { "name": "src/features/b.ts", "start": 40, "end": 57, "startLoc": { "line": 40, "column": 1 }, "endLoc": { "line": 57, "column": 2 } },
      "fragment": "function dup() { /* ... */ }"
    }
  ],
  "statistics": {}
}
```

`rules/baseline/fixtures/checker-outputs/jscpd.expected.json`:

```json
[
  { "firstFile": "src/features/a.ts", "firstStart": 10, "firstEnd": 27, "secondFile": "src/features/b.ts", "secondStart": 40, "secondEnd": 57, "lines": 18 }
]
```

- [ ] **Step 2: Write the failing test**

```js
// src/checkers/jscpd.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import jscpd, { parseJscpdReport, cloneViolations } from './jscpd.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const clones = parseJscpdReport(readFileSync(join(fixDir, 'jscpd.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'jscpd.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(clones) === JSON.stringify(expected));

// staged filtering: only clones touching a staged file produce a violation, pointed at the staged side
const stagedB = cloneViolations(clones, ['src/features/b.ts']);
assert('staged side selected', stagedB.length === 1 && stagedB[0].file === 'src/features/b.ts' && stagedB[0].line === 40);
assert('other side named', stagedB[0].text.includes('src/features/a.ts:10-27'));
const stagedNone = cloneViolations(clones, ['src/other.ts']);
assert('clone not touching staged → dropped', stagedNone.length === 0);
const full = cloneViolations(clones, null);
assert('full mode keeps all, points at first side', full.length === 1 && full[0].file === 'src/features/a.ts');
assert('violation shape', full[0].id === 'jscpd-clone' && full[0].severity === 'high' && full[0].category === 'duplication');

// detect
const root = mkdtempSync(join(tmpdir(), 'slopgate-jscpd-'));
assert('no bin → unavailable', jscpd.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/jscpd'), '');
assert('bin → available', jscpd.detect({ repoRoot: root }, {}).available === true);
assert('id', jscpd.id === 'jscpd');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 3: Run test to verify it fails**

Run: `node src/checkers/jscpd.test.mjs`
Expected: FAIL — `Cannot find module './jscpd.mjs'`

- [ ] **Step 4: Implement**

```js
// src/checkers/jscpd.mjs
/** jscpd adapter — copy-paste clones ("reimplemented instead of imported"). Scans the
 *  configured roots; in staged mode a clone counts only if one side overlaps a staged
 *  file (violation points at the staged side, excerpt names the other). */
import { readFileSync, rmSync, mkdtempSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseJscpdReport(jsonText) {
  const j = JSON.parse(jsonText);
  return (j.duplicates ?? []).map((d) => ({
    firstFile: d.firstFile.name,
    firstStart: d.firstFile.start ?? d.firstFile.startLoc?.line ?? 1,
    firstEnd: d.firstFile.end ?? d.firstFile.endLoc?.line ?? 1,
    secondFile: d.secondFile.name,
    secondStart: d.secondFile.start ?? d.secondFile.startLoc?.line ?? 1,
    secondEnd: d.secondFile.end ?? d.secondFile.endLoc?.line ?? 1,
    lines: d.lines,
  }));
}

/** @param {string[]|null} stagedFiles null = full mode (keep every clone, point at first side) */
export function cloneViolations(clones, stagedFiles, repoRoot = null) {
  const staged = stagedFiles ? new Set(stagedFiles) : null;
  const out = [];
  for (const c of clones) {
    let mine; let other; let line;
    if (!staged) {
      mine = c.firstFile; other = `${c.secondFile}:${c.secondStart}-${c.secondEnd}`; line = c.firstStart;
    } else if (staged.has(c.firstFile)) {
      mine = c.firstFile; other = `${c.secondFile}:${c.secondStart}-${c.secondEnd}`; line = c.firstStart;
    } else if (staged.has(c.secondFile)) {
      mine = c.secondFile; other = `${c.firstFile}:${c.firstStart}-${c.firstEnd}`; line = c.secondStart;
    } else continue;
    out.push({
      id: 'jscpd-clone', severity: 'high', category: 'duplication',
      file: mine, line,
      fullLine: repoRoot ? sourceLine(repoRoot, mine, line) : '',
      text: `duplicates ${other} (${c.lines} lines)`.slice(0, 90),
      resolution: 'Extract a shared util / import the existing implementation.',
    });
  }
  return out;
}

export default {
  id: 'jscpd',
  detect(config) {
    if (!localBin(config.repoRoot, 'jscpd')) return { available: false, reason: 'no local jscpd binary' };
    return { available: true };
  },
  run(config, cfg, { files = null } = {}) {
    const outDir = mkdtempSync(join(tmpdir(), 'slopgate-jscpd-'));
    const res = runTool(localBin(config.repoRoot, 'jscpd'), [
      ...config.rootsRel,
      '--min-tokens', String(cfg.minTokens ?? 50),
      '--reporters', 'json', '--output', outDir, '--silent',
    ], { cwd: config.repoRoot, timeout: (cfg.timeout ?? 60) * 1000 });
    try {
      if (!res.ok) return { violations: [], errors: [`jscpd failed: ${res.error}`] };
      const reportPath = join(outDir, 'jscpd-report.json');
      if (!existsSync(reportPath)) return { violations: [], errors: ['jscpd produced no report'] };
      let clones;
      try { clones = parseJscpdReport(readFileSync(reportPath, 'utf8')); }
      catch (e) { return { violations: [], errors: [`jscpd JSON parse error: ${e}`] }; }
      return { violations: cloneViolations(clones, files, config.repoRoot), errors: [] };
    } finally {
      rmSync(outDir, { recursive: true, force: true });
    }
  },
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `node src/checkers/jscpd.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 6: Commit**

```bash
git add src/checkers/jscpd.mjs src/checkers/jscpd.test.mjs rules/baseline/fixtures/checker-outputs/jscpd.json rules/baseline/fixtures/checker-outputs/jscpd.expected.json
git commit -m "feat(checkers): jscpd adapter — staged-side clone filtering"
```

---

### Task 8: dependency-cruiser checker

**Wave:** 2
**Blocks:** T11, T12
**Blocked by:** T2

**Files:**
- Create: `src/checkers/depcruise.mjs`
- Create: `rules/baseline/fixtures/checker-outputs/depcruise.json`
- Create: `rules/baseline/fixtures/checker-outputs/depcruise.expected.json`
- Test: `src/checkers/depcruise.test.mjs`

- [ ] **Step 1: Record the parser fixture**

`rules/baseline/fixtures/checker-outputs/depcruise.json`:

```json
{
  "summary": {
    "violations": [
      { "type": "cycle", "from": "src/a.ts", "to": "src/b.ts", "rule": { "severity": "error", "name": "no-circular" } },
      { "type": "module", "from": "src/orphan.ts", "to": "src/orphan.ts", "rule": { "severity": "warn", "name": "no-orphans" } },
      { "type": "dependency", "from": "src/ui/page.ts", "to": "src/db/client.ts", "rule": { "severity": "info", "name": "fyi-only" } }
    ],
    "error": 1, "warn": 1, "info": 1
  },
  "modules": []
}
```

`rules/baseline/fixtures/checker-outputs/depcruise.expected.json`:

```json
[
  { "rule": "no-circular", "severity": "error", "from": "src/a.ts", "to": "src/b.ts" },
  { "rule": "no-orphans", "severity": "warn", "from": "src/orphan.ts", "to": "src/orphan.ts" },
  { "rule": "fyi-only", "severity": "info", "from": "src/ui/page.ts", "to": "src/db/client.ts" }
]
```

- [ ] **Step 2: Write the failing test**

```js
// src/checkers/depcruise.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import depcruise, { parseDepcruiseOutput, depcruiseViolations } from './depcruise.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseDepcruiseOutput(readFileSync(join(fixDir, 'depcruise.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'depcruise.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));

const vios = depcruiseViolations(parsed);
assert('error → critical', vios[0].severity === 'critical' && vios[0].id === 'depcruise-no-circular');
assert('warn → high', vios[1].severity === 'high');
assert('info dropped', vios.length === 2);
assert('edge named in text', vios[0].text.includes('src/a.ts → src/b.ts'));
assert('category architecture', vios[0].category === 'architecture' && vios[0].file === 'src/a.ts' && vios[0].line === 1);

// detect: needs bin + a rules file
const root = mkdtempSync(join(tmpdir(), 'slopgate-dc-'));
const config = { repoRoot: root, configDir: join(root, '.slop-gate') };
mkdirSync(config.configDir, { recursive: true });
assert('no bin → unavailable', depcruise.detect(config, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/depcruise'), '');
assert('bin but no rules → unavailable', depcruise.detect(config, {}).available === false);
writeFileSync(join(config.configDir, 'depcruise.cjs'), 'module.exports={};');
assert('slop-gate rules file → available', depcruise.detect(config, {}).available === true);
assert('id', depcruise.id === 'depcruise');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 3: Run test to verify it fails**

Run: `node src/checkers/depcruise.test.mjs`
Expected: FAIL — `Cannot find module './depcruise.mjs'`

- [ ] **Step 4: Implement**

```js
// src/checkers/depcruise.mjs
/** dependency-cruiser adapter — the architecture gate: layer boundaries, cycles,
 *  orphans, encoded as rules in .slop-gate/depcruise.cjs (project-pinned). */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool } from './shared.mjs';

const SEVERITY_MAP = { error: 'critical', warn: 'high' }; // info → dropped

export function parseDepcruiseOutput(jsonText) {
  const j = JSON.parse(jsonText);
  return (j.summary?.violations ?? []).map((v) => ({
    rule: v.rule?.name ?? 'unknown', severity: v.rule?.severity ?? 'error', from: v.from, to: v.to,
  }));
}

export function depcruiseViolations(parsed) {
  const out = [];
  for (const v of parsed) {
    const severity = SEVERITY_MAP[v.severity];
    if (!severity) continue;
    out.push({
      id: `depcruise-${v.rule}`, severity, category: 'architecture',
      file: v.from, line: 1, fullLine: '',
      text: `${v.from} → ${v.to} violates ${v.rule}`.slice(0, 90),
      resolution: 'Respect the dependency rule — restructure the import, do not relax the rule.',
    });
  }
  return out;
}

function rulesFile(config, cfg) {
  const candidates = [
    cfg.rules ? join(config.configDir, cfg.rules) : null,
    join(config.configDir, 'depcruise.cjs'),
    join(config.repoRoot, '.dependency-cruiser.js'),
    join(config.repoRoot, '.dependency-cruiser.cjs'),
    join(config.repoRoot, '.dependency-cruiser.json'),
  ].filter(Boolean);
  return candidates.find(existsSync) ?? null;
}

export default {
  id: 'depcruise',
  detect(config, cfg) {
    if (!localBin(config.repoRoot, 'depcruise')) return { available: false, reason: 'no local depcruise binary' };
    if (!rulesFile(config, cfg)) return { available: false, reason: 'no depcruise rules file' };
    return { available: true };
  },
  run(config, cfg) {
    const res = runTool(localBin(config.repoRoot, 'depcruise'), [
      '--config', rulesFile(config, cfg), '--output-type', 'json', ...config.rootsRel,
    ], { cwd: config.repoRoot, timeout: (cfg.timeout ?? 60) * 1000 });
    if (!res.ok) return { violations: [], errors: [`depcruise failed: ${res.error}`] };
    let parsed;
    try { parsed = parseDepcruiseOutput(res.stdout); }
    catch (e) { return { violations: [], errors: [`depcruise JSON parse error: ${e}`] }; }
    return { violations: depcruiseViolations(parsed), errors: [] };
  },
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `node src/checkers/depcruise.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 6: Commit**

```bash
git add src/checkers/depcruise.mjs src/checkers/depcruise.test.mjs rules/baseline/fixtures/checker-outputs/depcruise.json rules/baseline/fixtures/checker-outputs/depcruise.expected.json
git commit -m "feat(checkers): dependency-cruiser adapter — architecture rules gate"
```

---

### Task 9: type-coverage checker

**Wave:** 2
**Blocks:** T11, T12
**Blocked by:** T2

**Files:**
- Create: `src/checkers/type-coverage.mjs`
- Create: `rules/baseline/fixtures/checker-outputs/type-coverage.txt`
- Create: `rules/baseline/fixtures/checker-outputs/type-coverage.expected.json`
- Test: `src/checkers/type-coverage.test.mjs`

- [ ] **Step 1: Record the parser fixture**

`rules/baseline/fixtures/checker-outputs/type-coverage.txt` (shape of `type-coverage --detail`; paths are emitted absolute or cwd-relative depending on version — parser handles both via the repoRoot strip):

```text
/repo/src/api/handler.ts:42:18: data
/repo/src/api/handler.ts:55:3: response
src/legacy/blob.ts:7:10: payload
2912 / 2930 99.38%
type-coverage success.
```

`rules/baseline/fixtures/checker-outputs/type-coverage.expected.json`:

```json
[
  { "file": "src/api/handler.ts", "line": 42, "name": "data" },
  { "file": "src/api/handler.ts", "line": 55, "name": "response" },
  { "file": "src/legacy/blob.ts", "line": 7, "name": "payload" }
]
```

- [ ] **Step 2: Write the failing test**

```js
// src/checkers/type-coverage.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import typeCoverage, { parseTypeCoverageOutput } from './type-coverage.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseTypeCoverageOutput(readFileSync(join(fixDir, 'type-coverage.txt'), 'utf8'), '/repo');
const expected = JSON.parse(readFileSync(join(fixDir, 'type-coverage.expected.json'), 'utf8'));
assert('fixture parses to expected (abs paths stripped, summary ignored)', JSON.stringify(parsed) === JSON.stringify(expected));
assert('empty → none', parseTypeCoverageOutput('100.00%\n', '/repo').length === 0);

const root = mkdtempSync(join(tmpdir(), 'slopgate-tc-'));
assert('no tsconfig → unavailable', typeCoverage.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'tsconfig.json'), '{}');
assert('no bin → unavailable', typeCoverage.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/type-coverage'), '');
assert('tsconfig + bin → available', typeCoverage.detect({ repoRoot: root }, {}).available === true);
assert('id', typeCoverage.id === 'type-coverage');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 3: Run test to verify it fails**

Run: `node src/checkers/type-coverage.test.mjs`
Expected: FAIL — `Cannot find module './type-coverage.mjs'`

- [ ] **Step 4: Implement**

```js
// src/checkers/type-coverage.mjs
/** type-coverage adapter — every implicitly-any expression is a violation; the ratchet
 *  baseline absorbs pre-existing ones, so coverage can only rise. No percent watermark:
 *  fingerprints give per-expression precision a percentage can't. */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseTypeCoverageOutput(stdout, repoRoot) {
  const out = [];
  for (const raw of stdout.split('\n')) {
    const m = /^(.+?\.(?:ts|tsx|mts|cts)):(\d+):(\d+):? (.*)$/.exec(raw.trim());
    if (!m) continue;
    let file = m[1].replace(/\\/g, '/');
    if (repoRoot && file.startsWith(`${repoRoot}/`)) file = file.slice(repoRoot.length + 1);
    out.push({ file, line: Number(m[2]), name: m[4] });
  }
  return out;
}

export default {
  id: 'type-coverage',
  detect(config) {
    if (!existsSync(join(config.repoRoot, 'tsconfig.json'))) return { available: false, reason: 'no tsconfig.json' };
    if (!localBin(config.repoRoot, 'type-coverage')) return { available: false, reason: 'no local type-coverage binary' };
    return { available: true };
  },
  run(config, cfg) {
    const res = runTool(localBin(config.repoRoot, 'type-coverage'), ['--detail'], {
      cwd: config.repoRoot, timeout: (cfg.timeout ?? 120) * 1000,
    });
    if (!res.ok) return { violations: [], errors: [`type-coverage failed: ${res.error}`] };
    const violations = parseTypeCoverageOutput(res.stdout, config.repoRoot).map((e) => ({
      id: 'type-coverage-uncovered', severity: 'high', category: 'types',
      file: e.file, line: e.line,
      fullLine: sourceLine(config.repoRoot, e.file, e.line),
      text: `implicitly any: ${e.name}`.slice(0, 90),
      resolution: 'Type this expression precisely.',
    }));
    return { violations, errors: [] };
  },
};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `node src/checkers/type-coverage.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 6: Commit**

```bash
git add src/checkers/type-coverage.mjs src/checkers/type-coverage.test.mjs rules/baseline/fixtures/checker-outputs/type-coverage.txt rules/baseline/fixtures/checker-outputs/type-coverage.expected.json
git commit -m "feat(checkers): type-coverage adapter — ratchet-backed any detection"
```

---

### Task 10: diff-shape checker

**Wave:** 2
**Blocks:** T11
**Blocked by:** —

**Files:**
- Create: `src/checkers/diff-shape.mjs`
- Test: `src/checkers/diff-shape.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/checkers/diff-shape.test.mjs
import diffShape, { concernGroups } from './diff-shape.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const rootsRel = ['src', 'workers/api/src'];
const groups = concernGroups(
  ['src/features/auth/a.ts', 'src/features/auth/b.ts', 'src/server/db.ts', 'workers/api/src/index.ts', 'README.md'],
  rootsRel,
);
assert('groups by root + first segment', groups.has('src/features') && groups.has('src/server') && groups.has('workers/api/src/(root)') === false);
assert('file directly under root → (root) group', concernGroups(['workers/api/src/index.ts'], rootsRel).has('workers/api/src/(root)'));
assert('non-root files ignored', !([...groups].some((g) => g.includes('README'))));
assert('group count', groups.size === 3);

const config = { rootsRel: ['src'] };
const wide = ['a', 'b', 'c', 'd', 'e', 'f'].map((d) => `src/${d}/x.ts`);
const r1 = diffShape.run(config, {}, { files: wide, mode: 'staged' });
assert('6 areas > default 5 → one violation', r1.violations.length === 1 && r1.violations[0].id === 'diff-shape-mixed-concerns');
assert('severity/category', r1.violations[0].severity === 'high' && r1.violations[0].category === 'hygiene');
const r2 = diffShape.run(config, {}, { files: wide.slice(0, 5), mode: 'staged' });
assert('5 areas ≤ 5 → clean', r2.violations.length === 0);
const r3 = diffShape.run(config, { maxDirs: 2 }, { files: wide.slice(0, 3), mode: 'staged' });
assert('maxDirs configurable', r3.violations.length === 1);
const r4 = diffShape.run(config, {}, { files: wide, mode: 'full' });
assert('full mode → never fires', r4.violations.length === 0);
assert('detect always available', diffShape.detect(config, {}).available === true);
assert('id', diffShape.id === 'diff-shape');

process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/checkers/diff-shape.test.mjs`
Expected: FAIL — `Cannot find module './diff-shape.mjs'`

- [ ] **Step 3: Implement**

```js
// src/checkers/diff-shape.mjs
/** diff-shape — staged set spanning too many concern areas = mixed-concern commit.
 *  Concern area = configured root + first path segment under it. Staged mode only;
 *  never enters the baseline (full-mode scans skip it by design). */

export function concernGroups(files, rootsRel) {
  const groups = new Set();
  for (const f of files) {
    const root = rootsRel.find((r) => f === r || f.startsWith(`${r}/`));
    if (!root) continue;
    const rest = f.slice(root.length + 1);
    const seg = rest.includes('/') ? rest.split('/')[0] : '(root)';
    groups.add(`${root}/${seg}`);
  }
  return groups;
}

export default {
  id: 'diff-shape',
  detect() { return { available: true }; },
  run(config, cfg, { files = [], mode } = {}) {
    if (mode !== 'staged') return { violations: [], errors: [] };
    const max = cfg.maxDirs ?? 5;
    const groups = concernGroups(files, config.rootsRel);
    if (groups.size <= max) return { violations: [], errors: [] };
    return {
      violations: [{
        id: 'diff-shape-mixed-concerns', severity: 'high', category: 'hygiene',
        file: files[0], line: 1, fullLine: '',
        text: `staged files span ${groups.size} areas (max ${max})`.slice(0, 90),
        resolution: `Split into focused commits. Areas: ${[...groups].slice(0, 8).join(', ')}`,
      }],
      errors: [],
    };
  },
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `node src/checkers/diff-shape.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 5: Commit**

```bash
git add src/checkers/diff-shape.mjs src/checkers/diff-shape.test.mjs
git commit -m "feat(checkers): diff-shape — mixed-concern staged commits"
```

---

### Task 11: Registry + gate tiers + ratchet integration + report

**Wave:** 3
**Blocks:** T14, T15
**Blocked by:** T1, T4, T5, T6, T7, T8, T9, T10

**Files:**
- Create: `src/checkers/index.mjs`
- Modify: `src/gate.mjs` (full rewrite below)
- Modify: `src/report.mjs` (signature + grouping)
- Test: `src/gate.tier.test.mjs`

- [ ] **Step 1: Write the failing test**

```js
// src/gate.tier.test.mjs
// Uses a fake checker injected via the registry seam to prove tier + ratchet behavior
// without external tools. Regex/ast paths already covered by self-test.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { resolveConfig } from './config.mjs';
import { collectViolations, runGate } from './gate.mjs';
import { CHECKERS } from './checkers/index.mjs';
import { writeBaseline } from './ratchet.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

assert('registry has 6 checkers', CHECKERS.length === 6
  && JSON.stringify(CHECKERS.map((c) => c.id)) === JSON.stringify(['tsc', 'knip', 'jscpd', 'depcruise', 'type-coverage', 'diff-shape']));
assert('every checker has detect+run', CHECKERS.every((c) => typeof c.detect === 'function' && typeof c.run === 'function'));

const repo = mkdtempSync(join(tmpdir(), 'slopgate-gate-'));
execSync('git init -q', { cwd: repo });
mkdirSync(join(repo, '.slop-gate'), { recursive: true });
mkdirSync(join(repo, 'src'), { recursive: true });
writeFileSync(join(repo, 'src/a.ts'), '// placeholder for now\nconst ok = 1;\n');
writeFileSync(join(repo, '.slop-gate/config.mjs'), `export default {
  roots: ['src'],
  baseline: ['no-stubs'],
  checkers: { 'fake': true },
};\n`);
execSync('git add src/a.ts', { cwd: repo });

const fake = {
  id: 'fake',
  detect: () => ({ available: true }),
  run: () => ({ violations: [{ id: 'fake-finding', severity: 'high', category: 'test', file: 'src/a.ts', line: 2, fullLine: 'const ok = 1;', text: 'fake', resolution: 'n/a' }], errors: [] }),
};
CHECKERS.push(fake);

const config = await resolveConfig(join(repo, '.slop-gate/config.mjs'));

// fast tier: regex fires, checker does NOT
const fast = collectViolations('staged', config, 'fast');
assert('fast tier: regex violation present', fast.violations.some((v) => v.id === 'no-stubs-placeholder'));
assert('fast tier: checker not run', !fast.violations.some((v) => v.id === 'fake-finding'));

// commit tier: checker runs, engine tagged, lineHash attached
const commit = collectViolations('staged', config, 'commit');
const fakeV = commit.violations.find((v) => v.id === 'fake-finding');
assert('commit tier: checker violation present', !!fakeV);
assert('checker violation engine-tagged', fakeV.engine === 'checker:fake');
assert('checker violation has lineHash', typeof fakeV.lineHash === 'string' && fakeV.lineHash.length === 40);

// disabled checker never runs even if registered
const noCfg = { ...config, checkers: {} };
const none = collectViolations('staged', noCfg, 'commit');
assert('unconfigured checker skipped silently', !none.violations.some((v) => v.id === 'fake-finding'));

// runGate commit tier blocks (exit 1), fast staged ignores baseline
const gateRes = runGate('staged', config);
assert('staged default = commit tier, blocks', gateRes.code === 1);

// baseline absorbs → exit 0
writeBaseline(config.baselinePath, gateRes.violations, '2026-06-10T00:00:00Z');
const after = runGate('staged', config);
assert('all baselined → exit 0', after.code === 0 && after.violations.length === 0);

// new violation on top of baseline → exit 1 with only the new one
writeFileSync(join(repo, 'src/a.ts'), '// placeholder for now\nconst ok = 1;\n// TODO: implement later\n');
execSync('git add src/a.ts', { cwd: repo });
const fresh = runGate('staged', config);
assert('only NEW violation fails', fresh.code === 1 && fresh.violations.length === 1 && fresh.violations[0].line === 3 && fresh.violations[0].id === 'no-stubs-placeholder');
assert('baselined ones still absorbed', !fresh.violations.some((v) => v.line === 1 || v.id === 'fake-finding'));

CHECKERS.pop();
rmSync(repo, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `node src/gate.tier.test.mjs`
Expected: FAIL — `Cannot find module './checkers/index.mjs'`

- [ ] **Step 3: Create the registry**

```js
// src/checkers/index.mjs
import tsc from './tsc.mjs';
import knip from './knip.mjs';
import jscpd from './jscpd.mjs';
import depcruise from './depcruise.mjs';
import typeCoverage from './type-coverage.mjs';
import diffShape from './diff-shape.mjs';

/** Commit-tier checkers, in execution order. Mutable on purpose: tests inject fakes. */
export const CHECKERS = [tsc, knip, jscpd, depcruise, typeCoverage, diffShape];
```

- [ ] **Step 4: Rewrite src/gate.mjs**

```js
// src/gate.mjs
import { runPatternScan, collectRegexViolations } from './regex-engine.mjs';
import { runAstGrepScan } from './ast-engine.mjs';
import { loadSuppressions, isSuppressed, lineHash } from './suppressions.mjs';
import { listSourceFiles } from './enumerate.mjs';
import { printGateReport } from './report.mjs';
import { loadBaseline, filterNew } from './ratchet.mjs';
import { CHECKERS } from './checkers/index.mjs';

/**
 * Collect raw violations (no suppressions / severity / ratchet filtering).
 * @param {'file'|'staged'|'full'} mode  'full' walks configured roots (baseline snapshot)
 * @param {'fast'|'commit'} tier  checkers run in commit tier only
 * @returns {{ violations:any[], notices:string[] }}
 */
export function collectViolations(mode, config, tier) {
  const opts = mode === 'staged' ? { staged: true } : mode === 'file' ? { file: config._fileTarget } : {};
  const files = listSourceFiles(config, opts);
  const notices = [];
  if (files.length === 0 && mode !== 'full') return { violations: [], notices };

  const violations = collectRegexViolations(config, runPatternScan(config, opts));

  const ast = runAstGrepScan(config, mode === 'full' ? null : files);
  if (!ast.available) notices.push(ast.errors.join('; '));
  else for (const e of ast.errors) notices.push(`ast-grep: ${e}`);
  for (const v of ast.violations) {
    if (config.astDisable.has(v.id)) continue;
    violations.push({ ...v, lineHash: lineHash(v.fullLine) });
  }

  if (tier === 'commit') {
    for (const checker of CHECKERS) {
      const cfg = config.checkers[checker.id];
      if (!cfg) continue; // disabled / unconfigured
      const det = checker.detect(config, cfg);
      if (!det.available) { notices.push(`skipped: ${checker.id} (${det.reason})`); continue; }
      const res = checker.run(config, cfg, { files: mode === 'full' ? null : files, mode });
      for (const e of res.errors) notices.push(`${checker.id}: ${e}`);
      for (const v of res.violations) {
        violations.push({ ...v, engine: `checker:${checker.id}`, lineHash: lineHash(v.fullLine ?? '') });
      }
    }
  }
  return { violations, notices };
}

/**
 * @param {'file'|'staged'} mode
 * @param {{ tier?: 'fast'|'commit' }} [opts]  default: staged→commit, file→fast
 * @returns {{ violations:any[], code:number }}
 */
export function runGate(mode, config, { tier } = {}) {
  const effTier = tier ?? (mode === 'staged' ? 'commit' : 'fast');
  const { violations: collected, notices } = collectViolations(mode, config, effTier);
  for (const n of notices) process.stderr.write(`⚠ SLOP-GATE: ${n}\n`);

  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);

  let violations = collected
    .filter((v) => allow.has(v.severity))
    .filter((v) => !isSuppressed(sup.entries, v));

  let baselinedCount = 0;
  if (effTier === 'commit') {
    const bl = loadBaseline(config.baselinePath);
    if (bl.error) process.stderr.write(`⚠ SLOP-GATE: baseline.json malformed (${bl.error}) — treating as EMPTY (everything is new)\n`);
    if (bl.missing && violations.length) {
      process.stderr.write(`⚠ SLOP-GATE: no baseline — run: slop-gate baseline --config <config> to absorb pre-existing violations\n`);
    }
    ({ fresh: violations, baselinedCount } = filterNew(violations, bl.entries));
  }

  if (violations.length === 0) {
    if (baselinedCount > 0) process.stderr.write(`SLOP-GATE: clean (${baselinedCount} pre-existing baselined violation(s) ignored)\n`);
    return { violations, code: 0 };
  }
  printGateReport(violations, mode, { baselinedCount });
  return { violations, code: 1 };
}
```

- [ ] **Step 5: Update src/report.mjs**

Replace the function signature and add source grouping + footer (keep all existing color codes / framing):

```js
// src/report.mjs
export function printGateReport(violations, mode, { baselinedCount = 0 } = {}) {
  const R = '\x1b[31m'; const Y = '\x1b[33m'; const B = '\x1b[1m'; const D = '\x1b[2m'; const Z = '\x1b[0m';
  const title = mode === 'file'
    ? 'SLOP-GATE — VIOLATIONS IN EDITED FILE               '
    : 'VIOLATIONS IN STAGED FILES — COMMIT BLOCKED         ';
  process.stderr.write(`\n${B}${R}╔═ SLOP-GATE ═════════════════════════════════════════╗${Z}\n`);
  process.stderr.write(`${B}${R}║ ${title}║${Z}\n`);
  process.stderr.write(`${B}${R}╚═════════════════════════════════════════════════════╝${Z}\n\n`);

  const order = (v) => (v.engine ?? 'regex');
  const sorted = [...violations].sort((a, b) => order(a).localeCompare(order(b)) || a.file.localeCompare(b.file) || a.line - b.line);
  let currentGroup = null;
  for (const v of sorted) {
    const group = order(v);
    if (group !== currentGroup) {
      currentGroup = group;
      process.stderr.write(`${B}── ${group} ──${Z}\n`);
    }
    const C = v.severity === 'critical' ? R : Y;
    process.stderr.write(`${B}${C}[${v.severity.toUpperCase()}]${Z} ${B}${v.id}${Z} ${D}${v.file}:${v.line}${Z}\n`);
    process.stderr.write(`  ${D}×${Z} ${v.text}\n`);
    process.stderr.write(`  ${B}✓${Z} ${v.resolution}\n\n`);
  }
  const files = new Set(violations.map((v) => v.file)).size;
  const tail = mode === 'file' ? 'Fix now while context is hot.' : 'Fix → retry commit.';
  process.stderr.write(`${B}${violations.length} violation(s) in ${files} file(s). ${tail}${Z}\n`);
  if (baselinedCount > 0) process.stderr.write(`${D}${baselinedCount} pre-existing (baselined) violation(s) ignored.${Z}\n`);
  process.stderr.write(`False positive? NEVER edit suppressions.json yourself — ask the user via AskUserQuestion.\n\n`);
}
```

- [ ] **Step 6: Run tests**

Run: `node src/gate.tier.test.mjs && npm run self-test`
Expected: all PASS, exit 0; self-test unchanged (regex violations carry `engine: 'regex'` already — verify the field exists in `collectRegexViolations`; it does: `engine: 'regex'`).

- [ ] **Step 7: Commit**

```bash
git add src/checkers/index.mjs src/gate.mjs src/report.mjs src/gate.tier.test.mjs
git commit -m "feat(gate): commit tier runs checkers + ratchet baseline; grouped report"
```

---

### Task 12: Self-test parser-fixture stage

**Wave:** 3
**Blocks:** —
**Blocked by:** T5–T9

**Files:**
- Modify: `src/selftest.mjs`

- [ ] **Step 1: Add the parser-fixture stage**

Append to `src/selftest.mjs` — new imports at top:

```js
import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseTscOutput } from './checkers/tsc.mjs';
import { parseKnipOutput } from './checkers/knip.mjs';
import { parseJscpdReport } from './checkers/jscpd.mjs';
import { parseDepcruiseOutput } from './checkers/depcruise.mjs';
import { parseTypeCoverageOutput } from './checkers/type-coverage.mjs';
```

Inside `runSelfTest`, before the final `return failed ? 1 : 0;`, add:

```js
  // checker parser fixtures: recorded real tool outputs must parse to expected shapes.
  // Catches tool-output-format drift without invoking the tools.
  const fixDir = join(dirname(fileURLToPath(import.meta.url)), '../rules/baseline/fixtures/checker-outputs');
  const PARSER_FIXTURES = [
    { id: 'tsc', input: 'tsc.txt', expected: 'tsc.expected.json', parse: (t) => parseTscOutput(t) },
    { id: 'knip', input: 'knip.json', expected: 'knip.expected.json', parse: (t) => parseKnipOutput(t) },
    { id: 'jscpd', input: 'jscpd.json', expected: 'jscpd.expected.json', parse: (t) => parseJscpdReport(t) },
    { id: 'depcruise', input: 'depcruise.json', expected: 'depcruise.expected.json', parse: (t) => parseDepcruiseOutput(t) },
    { id: 'type-coverage', input: 'type-coverage.txt', expected: 'type-coverage.expected.json', parse: (t) => parseTypeCoverageOutput(t, '/repo') },
  ];
  for (const f of PARSER_FIXTURES) {
    const inPath = join(fixDir, f.input);
    const expPath = join(fixDir, f.expected);
    if (!existsSync(inPath) || !existsSync(expPath)) { console.error(`FAIL parser ${f.id}: fixture missing`); failed++; continue; }
    try {
      const got = JSON.stringify(f.parse(readFileSync(inPath, 'utf8')));
      const want = JSON.stringify(JSON.parse(readFileSync(expPath, 'utf8')));
      if (got !== want) { console.error(`FAIL parser ${f.id}: parsed output != expected`); failed++; }
      else console.error(`OK parser ${f.id}`);
    } catch (e) { console.error(`FAIL parser ${f.id}: ${e}`); failed++; }
  }
```

- [ ] **Step 2: Run self-test**

Run: `npm run self-test`
Expected: existing OK lines + 5 new `OK parser <id>` lines, exit 0

- [ ] **Step 3: Prove it catches drift**

Temporarily change `"line": 12` to `"line": 13` in `rules/baseline/fixtures/checker-outputs/tsc.expected.json`, run `npm run self-test`, expect `FAIL parser tsc` + exit 1. Revert the change, re-run, expect exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/selftest.mjs
git commit -m "feat(selftest): checker parser-fixture stage — catches tool output drift"
```

---

### Task 13: init — checkers scaffold, depcruise starter, hook install

**Wave:** 3
**Blocks:** —
**Blocked by:** T3

**Files:**
- Modify: `src/init.mjs` (`formatConfig`, `runInit`)
- Modify: `src/init.test.mjs` (extend existing assertions)

- [ ] **Step 1: Extend formatConfig**

`formatConfig` gains a `detected.checkers` param. Replace the function with:

```js
/** @param {{ roots: string[], exts: string[], skipDirs: string[], checkers: Record<string, any> }} detected */
function formatConfig(detected) {
  const checkerLines = Object.entries(detected.checkers)
    .map(([k, v]) => `    '${k}': ${JSON.stringify(v)},`).join('\n');
  return `// generated by slop-gate init — review roots + add project rule packs (see convention-sources.json).
// slop-gate project config. Engine is global+auto-latest; THIS file is pinned per project.
export default {
  roots: ${JSON.stringify(detected.roots)},
  exts: ${JSON.stringify(detected.exts)},
  skipDirs: ${JSON.stringify(detected.skipDirs)},

  // baseline packs this project OPTS INTO (nothing fires until listed)
  baseline: ['no-stubs', 'ts-suppress', 'as-any'],

  // project-owned rule packs (pinned, in this repo)
  rules: [],                      // e.g. ['./rules/my-rule.mjs']
  astRules: './rules/ast',        // dir of *.yml (optional)
  astDisable: [],                 // ast rule ids to silence in this project

  // commit-tier checkers (detected at init; false/absent = off)
  checkers: {
${checkerLines}
  },

  gate: { file: ['critical', 'high'], staged: ['critical', 'high'] },
  suppressions: './suppressions.json',
  fixtures: './fixtures',
};
`;
}
```

- [ ] **Step 2: Add checker detection + depcruise starter + hook install to runInit**

Add near the top of `src/init.mjs`:

```js
import { installPreCommitHook } from './install-hooks.mjs';

const DEPCRUISE_STARTER = `// generated by slop-gate init — universal rules; add project layer rules here.
module.exports = {
  forbidden: [
    { name: 'no-circular', severity: 'error', from: {}, to: { circular: true } },
    { name: 'no-orphans', severity: 'warn', from: { orphan: true, pathNot: ['\\\\.d\\\\.ts$'] }, to: {} },
  ],
  options: { doNotFollow: { path: 'node_modules' }, tsPreCompilationDeps: true },
};
`;

/** @param {string} targetDir */
export function detectCheckers(targetDir) {
  const bin = (name) => existsSync(join(targetDir, 'node_modules/.bin', name));
  const checkers = {};
  if (existsSync(join(targetDir, 'tsconfig.json')) && bin('tsc')) checkers.tsc = true;
  if (bin('knip')) checkers.knip = true;
  if (bin('jscpd')) checkers.jscpd = { minTokens: 50 };
  if (bin('depcruise')) checkers.depcruise = true;
  if (bin('type-coverage')) checkers['type-coverage'] = true;
  checkers['diff-shape'] = { maxDirs: 5 };
  return checkers;
}
```

In `runInit`, change the scaffold block:

```js
  const checkers = detectCheckers(targetDir);

  if (!configExists) {
    mkdirSync(join(base, 'fixtures/src'), { recursive: true });
    writeFileSync(configPath, formatConfig({ roots, exts, skipDirs, checkers }));
    writeFileSync(
      join(base, 'suppressions.json'),
      `${JSON.stringify({ version: 1, entries: [] }, null, 2)}\n`,
    );
  }
  const depcruisePath = join(base, 'depcruise.cjs');
  if (checkers.depcruise && !existsSync(depcruisePath)) writeFileSync(depcruisePath, DEPCRUISE_STARTER);

  let hookAction = 'skipped (not a git repo)';
  try { hookAction = installPreCommitHook(targetDir).action; } catch { /* not a git repo */ }
```

In the summary output, after the `settings:` line add:

```js
    process.stdout.write(`checkers:  ${JSON.stringify(Object.keys(checkers))}\n`);
    process.stdout.write(`pre-commit hook: ${hookAction}\n`);
```

And append a 5th NEXT STEP:

```js
    process.stdout.write('  5. Run: slop-gate baseline --config .slop-gate/config.mjs (absorb pre-existing violations)\n');
```

- [ ] **Step 3: Extend init test**

In `src/init.test.mjs`, after the existing assertions on the generated config (locate the block that reads the generated `config.mjs`), add — and make the fixture a git repo so hook install is exercised: in `setupFixture()` after the mkdirs add `execSync('git init -q', { cwd: FIXTURE });` (import `execSync` from `node:child_process` at top). Then add assertions:

```js
const cfgText = readFileSync(join(FIXTURE, '.slop-gate/config.mjs'), 'utf8');
assert('config has checkers block', cfgText.includes('checkers: {'));
assert("config has diff-shape default", cfgText.includes("'diff-shape': {\"maxDirs\":5}"));
assert('config has astDisable', cfgText.includes('astDisable: []'));
assert('pre-commit hook installed', existsSync(join(FIXTURE, '.git/hooks/pre-commit')));
```

- [ ] **Step 4: Run tests**

Run: `node src/init.test.mjs`
Expected: all PASS (existing + new), exit 0

- [ ] **Step 5: Commit**

```bash
git add src/init.mjs src/init.test.mjs
git commit -m "feat(init): checker detection, depcruise starter, pre-commit install, baseline hint"
```

---

### Task 14: CLI — --tier, baseline, install-hooks

**Wave:** 4
**Blocks:** T15
**Blocked by:** T1, T3, T4, T11

**Files:**
- Modify: `src/cli.mjs` (full rewrite below)

- [ ] **Step 1: Rewrite src/cli.mjs**

```js
// src/cli.mjs
import { existsSync } from 'node:fs';
import { resolveConfig } from './config.mjs';
import { runGate, collectViolations } from './gate.mjs';
import { runSelfTest } from './selftest.mjs';
import { runInit } from './init.mjs';
import { loadSuppressions, isSuppressed } from './suppressions.mjs';
import { loadBaseline, writeBaseline, writeBaselineRaw, fingerprintViolation } from './ratchet.mjs';
import { installPreCommitHook } from './install-hooks.mjs';

const args = process.argv.slice(2);
const has = (f) => args.includes(f);
const valOf = (f) => { const i = args.indexOf(f); return i === -1 ? null : args[i + 1]; };

/** Full-repo commit-tier snapshot, filtered like the gate filters (severity + suppressions). */
function snapshotViolations(config) {
  const { violations, notices } = collectViolations('full', config, 'commit');
  for (const n of notices) process.stderr.write(`⚠ SLOP-GATE: ${n}\n`);
  const allow = new Set(config.gate.staged ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);
  return violations.filter((v) => allow.has(v.severity)).filter((v) => !isSuppressed(sup.entries, v));
}

async function requireConfig() {
  const configPath = valOf('--config');
  if (!configPath) { process.stderr.write('slop-gate: --config <path> required\n'); process.exit(2); }
  return resolveConfig(configPath);
}

async function main() {
  if (has('init')) {
    const dir = valOf('init') || process.cwd();
    process.exit(runInit(dir));
  }

  if (has('install-hooks')) {
    const config = await requireConfig();
    const { action, path } = installPreCommitHook(config.repoRoot);
    process.stdout.write(`slop-gate: pre-commit hook ${action} (${path})\n`);
    process.exit(0);
  }

  if (has('baseline')) {
    const config = await requireConfig();
    const exists = existsSync(config.baselinePath);

    if (has('--prune') && !has('--update')) {
      // drop entries whose fingerprint no longer occurs; never adds new ones
      const bl = loadBaseline(config.baselinePath);
      if (bl.error || bl.missing) { process.stderr.write('slop-gate: no valid baseline to prune\n'); process.exit(2); }
      const current = new Set(snapshotViolations(config).map(fingerprintViolation));
      const kept = Object.fromEntries(Object.entries(bl.entries).filter(([fp]) => current.has(fp)));
      const dropped = Object.keys(bl.entries).length - Object.keys(kept).length;
      writeBaselineRaw(config.baselinePath, kept, new Date().toISOString());
      process.stdout.write(`slop-gate: baseline pruned — ${dropped} resolved entr${dropped === 1 ? 'y' : 'ies'} removed, ${Object.keys(kept).length} kept\n`);
      process.exit(0);
    }

    if (exists && !has('--update')) {
      process.stderr.write('slop-gate: baseline.json exists — use `baseline --update` to re-snapshot (this re-absorbs ALL current violations) or `baseline --prune` to drop resolved entries\n');
      process.exit(2);
    }
    const snap = snapshotViolations(config);
    const n = writeBaseline(config.baselinePath, snap, new Date().toISOString());
    process.stdout.write(`slop-gate: baseline written — ${n} entr${n === 1 ? 'y' : 'ies'} → ${config.baselinePath}\n`);
    process.exit(0);
  }

  const config = await requireConfig();
  if (has('--self-test')) process.exit(runSelfTest(config));

  const tierFlag = valOf('--tier'); // 'fast' | 'commit' | null (default by mode)
  if (tierFlag && tierFlag !== 'fast' && tierFlag !== 'commit') {
    process.stderr.write('slop-gate: --tier must be fast|commit\n'); process.exit(2);
  }
  if (has('--staged')) process.exit(runGate('staged', config, { tier: tierFlag ?? undefined }).code);
  const fileTarget = valOf('--file');
  if (fileTarget) { config._fileTarget = fileTarget; process.exit(runGate('file', config, { tier: tierFlag ?? undefined }).code); }

  process.stderr.write('slop-gate: no mode (use --staged | --file <p> | --self-test | init [dir] | baseline [--update|--prune] | install-hooks)\n');
  process.exit(2);
}
main().catch((e) => { process.stderr.write(`slop-gate: ${e?.stack || e}\n`); process.exit(1); });
```

- [ ] **Step 2: Verify manually against this repo's own baseline config**

```bash
npm run self-test                                      # still exits 0
node bin/slop-gate --config rules/baseline/selftest.config.mjs --staged   # exits 0 (nothing staged)
node bin/slop-gate --config rules/baseline/selftest.config.mjs --tier bogus --staged ; echo "exit=$?"   # prints usage error, exit=2
```

- [ ] **Step 3: Commit**

```bash
git add src/cli.mjs
git commit -m "feat(cli): --tier flag, baseline/--update/--prune, install-hooks"
```

---

### Task 15: End-to-end smoke test

**Wave:** 5
**Blocks:** —
**Blocked by:** T11, T13, T14

**Files:**
- Test: `src/gate.e2e.test.mjs`

- [ ] **Step 1: Write the e2e test**

```js
// src/gate.e2e.test.mjs
// Full loop through bin/slop-gate as a child process: init → violation blocks →
// baseline absorbs → new violation blocks again → prune. No external tools needed
// (regex pack only) — checker plumbing is covered by gate.tier.test.mjs.
import { mkdirSync, writeFileSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { execSync, spawnSync } from 'node:child_process';

const BIN = '/home/user/Projects/slop-gate/bin/slop-gate';
const REPO = '/home/user/Projects/slop-gate/.tmp-e2e';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }
function gate(...extra) {
  return spawnSync('node', [BIN, '--config', join(REPO, '.slop-gate/config.mjs'), ...extra], { encoding: 'utf8', cwd: REPO });
}

rmSync(REPO, { recursive: true, force: true });
mkdirSync(join(REPO, 'src'), { recursive: true });
execSync('git init -q && git config user.email t@t && git config user.name t', { cwd: REPO });
mkdirSync(join(REPO, '.slop-gate'), { recursive: true });
writeFileSync(join(REPO, '.slop-gate/config.mjs'), `export default {
  roots: ['src'],
  baseline: ['no-stubs'],
  checkers: { 'diff-shape': { maxDirs: 5 } },
};\n`);

// 1. staged violation blocks (commit tier default), with no-baseline hint
writeFileSync(join(REPO, 'src/a.ts'), 'export const a = 1; // placeholder for now\n');
execSync('git add src/a.ts', { cwd: REPO });
const r1 = gate('--staged');
assert('violation → exit 1', r1.status === 1);
assert('no-baseline hint shown', r1.stderr.includes('run: slop-gate baseline'));

// 2. fast tier on same staged set skips ratchet machinery but still reports
const r2 = gate('--staged', '--tier', 'fast');
assert('fast tier also blocks raw violation', r2.status === 1);

// 3. baseline absorbs it
const r3 = gate('baseline');
assert('baseline cmd exit 0', r3.status === 0 && existsSync(join(REPO, '.slop-gate/baseline.json')));
const r4 = gate('--staged');
assert('baselined → exit 0', r4.status === 0);
assert('baselined count reported', r4.stderr.includes('baselined'));

// 4. baseline refuses accidental overwrite
const r5 = gate('baseline');
assert('second baseline refused without --update', r5.status === 2);

// 5. NEW violation still blocks
writeFileSync(join(REPO, 'src/b.ts'), 'export const b = 1; // TODO: implement\n');
execSync('git add src/b.ts', { cwd: REPO });
const r6 = gate('--staged');
assert('new violation → exit 1', r6.status === 1);
assert('report names only new file', r6.stderr.includes('src/b.ts') && !r6.stderr.match(/\[CRITICAL\].*src\/a\.ts/));

// 6. fix the old one, prune shrinks baseline
writeFileSync(join(REPO, 'src/a.ts'), 'export const a = 1;\n');
const r7 = gate('baseline', '--prune');
assert('prune reports removal', r7.status === 0 && /1 resolved entry removed/.test(r7.stdout));
const bl = JSON.parse(readFileSync(join(REPO, '.slop-gate/baseline.json'), 'utf8'));
assert('baseline emptied', Object.keys(bl.entries).length === 0);

rmSync(REPO, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
```

- [ ] **Step 2: Run it**

Run: `node src/gate.e2e.test.mjs`
Expected: all PASS, exit 0

- [ ] **Step 3: Run the full suite + self-test one last time**

```bash
for t in src/ratchet.test.mjs src/checkers/shared.test.mjs src/install-hooks.test.mjs src/config.checkers.test.mjs src/checkers/tsc.test.mjs src/checkers/knip.test.mjs src/checkers/jscpd.test.mjs src/checkers/depcruise.test.mjs src/checkers/type-coverage.test.mjs src/checkers/diff-shape.test.mjs src/gate.tier.test.mjs src/init.test.mjs src/gate.e2e.test.mjs; do node "$t" || echo "SUITE FAIL: $t"; done
npm run self-test
```
Expected: zero `SUITE FAIL`, self-test exit 0.

- [ ] **Step 4: Commit**

```bash
git add src/gate.e2e.test.mjs
git commit -m "test: e2e — init/gate/baseline/prune loop through bin"
```

---

## Plan 2 / Plan 3 pointers (separate plans, after this ships)

- **Plan 2 — rule packs** (spec §3.5): semantic ast pack (empty-catch, swallowed-error, console-debug-left), depth pack (pass-through-fn, delegating-wrapper), test-slop pack (+ regex `includeGlobs` engine support + edit-hook test-file change), comment-slop `no-stubs` additions, security ast rules.
- **Plan 3 — audit command** (spec §3.9): hotspots, module shape from depcruise graph, co-change coupling, ratchet progress; `audit` CLI subcommand.
