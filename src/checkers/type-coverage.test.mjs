// src/checkers/type-coverage.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import typeCoverage, { parseTypeCoverageOutput } from './type-coverage.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseTypeCoverageOutput(readFileSync(join(fixDir, 'type-coverage.txt'), 'utf8'), '/repo');
const expected = JSON.parse(readFileSync(join(fixDir, 'type-coverage.expected.json'), 'utf8'));
legacyAssert('fixture parses to expected (abs paths stripped, summary ignored)', JSON.stringify(parsed) === JSON.stringify(expected));
legacyAssert('empty → none', parseTypeCoverageOutput('100.00%\n', '/repo').length === 0);

const root = mkdtempSync(join(tmpdir(), 'slopgate-tc-'));
legacyAssert('no tsconfig → unavailable', typeCoverage.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'tsconfig.json'), '{}');
legacyAssert('no bin → unavailable', typeCoverage.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/type-coverage'), '');
legacyAssert('tsconfig + bin → available', typeCoverage.detect({ repoRoot: root }, {}).available === true);
legacyAssert('id', typeCoverage.id === 'type-coverage');

rmSync(root, { recursive: true, force: true });

test('legacy type-coverage helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('type-coverage run() is async', async () => {
  const tcMod = (await import('./type-coverage.mjs')).default;
  const p = tcMod.run({ repoRoot: process.cwd() }, {});
  assert.equal(typeof p.then, 'function');
  await p;
});
