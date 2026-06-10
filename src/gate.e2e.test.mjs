// src/gate.e2e.test.mjs
// Full loop through bin/slop-gate as a child process: init → violation blocks →
// baseline absorbs → new violation blocks again → prune. No external tools needed
// (regex pack only) — checker plumbing is covered by gate.tier.test.mjs.
import { mkdirSync, writeFileSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync, spawnSync } from 'node:child_process';

const HERE = dirname(fileURLToPath(import.meta.url));
const BIN = join(HERE, '../bin/slop-gate');
const REPO = join(HERE, '../.tmp-e2e');

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
