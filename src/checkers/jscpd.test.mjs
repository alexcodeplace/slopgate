// src/checkers/jscpd.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import jscpd, { parseJscpdReport, cloneViolations } from './jscpd.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const clones = parseJscpdReport(readFileSync(join(fixDir, 'jscpd.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'jscpd.expected.json'), 'utf8'));
legacyAssert('fixture parses to expected', JSON.stringify(clones) === JSON.stringify(expected));

// staged filtering: only clones touching a staged file produce a violation, pointed at the staged side
const stagedB = cloneViolations(clones, ['src/features/b.ts']);
legacyAssert('staged side selected', stagedB.length === 1 && stagedB[0].file === 'src/features/b.ts' && stagedB[0].line === 40);
legacyAssert('other side named', stagedB[0].text.includes('src/features/a.ts:10-27'));
const stagedNone = cloneViolations(clones, ['src/other.ts']);
legacyAssert('clone not touching staged → dropped', stagedNone.length === 0);
const full = cloneViolations(clones, null);
legacyAssert('full mode keeps all, points at first side', full.length === 1 && full[0].file === 'src/features/a.ts');
legacyAssert('violation shape', full[0].id === 'jscpd-clone' && full[0].severity === 'high' && full[0].category === 'duplication');

// detect
const root = mkdtempSync(join(tmpdir(), 'slopgate-jscpd-'));
legacyAssert('no bin → unavailable', jscpd.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/jscpd'), '');
legacyAssert('bin → available', jscpd.detect({ repoRoot: root }, {}).available === true);
legacyAssert('id', jscpd.id === 'jscpd');

rmSync(root, { recursive: true, force: true });

test('legacy jscpd helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('jscpd run() is async and never throws when binary missing', async () => {
  const jscpd = (await import('./jscpd.mjs')).default;
  const p = jscpd.run({ repoRoot: '/nonexistent-xyz', rootsRel: ['src'] }, {}, { files: null });
  assert.equal(typeof p.then, 'function');
  const r = await p;
  assert.ok(Array.isArray(r.errors));
});
