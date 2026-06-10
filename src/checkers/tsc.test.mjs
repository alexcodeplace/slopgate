// src/checkers/tsc.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import tsc, { parseTscOutput, resolveTscBin } from './tsc.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseTscOutput(readFileSync(join(fixDir, 'tsc.txt'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'tsc.expected.json'), 'utf8'));
legacyAssert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));
legacyAssert('empty output → no errors', parseTscOutput('').length === 0);

// detect
const pathHasTsc = spawnSync('tsc', ['--version'], { encoding: 'utf8' }).status === 0;

const root = mkdtempSync(join(tmpdir(), 'slopgate-tsc-'));
legacyAssert('no tsconfig → unavailable', tsc.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'tsconfig.json'), '{}');
legacyAssert('no local tsc → PATH fallback decides', tsc.detect({ repoRoot: root }, {}).available === pathHasTsc);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '');
legacyAssert('tsconfig + local bin → available', tsc.detect({ repoRoot: root }, {}).available === true);
legacyAssert('custom tsconfig honored', tsc.detect({ repoRoot: root }, { tsconfig: 'tsconfig.app.json' }).available === false);

// array form (spec 3.4: monorepo support)
writeFileSync(join(root, 'tsconfig.app.json'), '{}');
legacyAssert('array: all exist → available',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'tsconfig.app.json'] }).available === true);
legacyAssert('array: one missing → unavailable',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'nope.json'] }).available === false);
legacyAssert('array: missing reason names the file',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'nope.json'] }).reason === 'no nope.json');
legacyAssert('id', tsc.id === 'tsc');

rmSync(root, { recursive: true, force: true });

test('legacy tsc helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('resolveTscBin reports source', () => {
  const r = resolveTscBin(process.cwd());
  if (r !== null) { assert.ok(['local','path'].includes(r.source)); assert.equal(typeof r.bin, 'string'); }
});

test('tsc run() is async', async () => {
  const tscMod = (await import('./tsc.mjs')).default;
  const p = tscMod.run({ repoRoot: process.cwd() }, { tsconfig: 'tsconfig.json' });
  assert.equal(typeof p.then, 'function');
  await p;
});
