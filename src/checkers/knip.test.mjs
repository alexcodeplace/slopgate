// src/checkers/knip.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import knip, { parseKnipOutput } from './knip.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseKnipOutput(JSON.parse(readFileSync(join(fixDir, 'knip.json'), 'utf8')));
const expected = JSON.parse(readFileSync(join(fixDir, 'knip.expected.json'), 'utf8'));
legacyAssert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));
legacyAssert('empty report → none', parseKnipOutput({ files: [], issues: [] }).length === 0);

const root = mkdtempSync(join(tmpdir(), 'slopgate-knip-'));
writeFileSync(join(root, 'package.json'), '{}');
legacyAssert('no bin → unavailable', knip.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/knip'), '');
legacyAssert('bin but no knip config → unavailable', knip.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'knip.json'), '{}');
legacyAssert('bin + knip.json → available', knip.detect({ repoRoot: root }, {}).available === true);
rmSync(join(root, 'knip.json'));
writeFileSync(join(root, 'package.json'), '{"knip":{}}');
legacyAssert('pkg.knip counts as config', knip.detect({ repoRoot: root }, {}).available === true);
legacyAssert('id', knip.id === 'knip');

rmSync(root, { recursive: true, force: true });

test('legacy knip helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('knip run() returns a promise and never throws on tool failure', async () => {
  const knipMod = (await import('./knip.mjs')).default;
  const p = knipMod.run({ repoRoot: '/nonexistent-xyz', checkers: {} }, {});
  assert.equal(typeof p.then, 'function');
  const r = await p;
  assert.ok(Array.isArray(r.errors));
});
