// src/checkers/depcruise.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import depcruise, { parseDepcruiseOutput, depcruiseViolations } from './depcruise.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseDepcruiseOutput(JSON.parse(readFileSync(join(fixDir, 'depcruise.json'), 'utf8')));
const expected = JSON.parse(readFileSync(join(fixDir, 'depcruise.expected.json'), 'utf8'));
legacyAssert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));

const vios = depcruiseViolations(parsed);
legacyAssert('error → critical', vios[0].severity === 'critical' && vios[0].id === 'depcruise-no-circular');
legacyAssert('warn → high', vios[1].severity === 'high');
legacyAssert('info dropped', vios.length === 2);
legacyAssert('edge named in text', vios[0].text.includes('src/a.ts → src/b.ts'));
legacyAssert('category architecture', vios[0].category === 'architecture' && vios[0].file === 'src/a.ts' && vios[0].line === 1);

// detect: needs bin + a rules file
const root = mkdtempSync(join(tmpdir(), 'slopgate-dc-'));
const config = { repoRoot: root, configDir: join(root, '.slopgate') };
mkdirSync(config.configDir, { recursive: true });
legacyAssert('no bin → unavailable', depcruise.detect(config, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/depcruise'), '');
legacyAssert('bin but no rules → unavailable', depcruise.detect(config, {}).available === false);
writeFileSync(join(config.configDir, 'depcruise.cjs'), 'module.exports={};');
legacyAssert('slopgate rules file → available', depcruise.detect(config, {}).available === true);
legacyAssert('id', depcruise.id === 'depcruise');

rmSync(root, { recursive: true, force: true });

test('legacy depcruise helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('depcruiseViolations skips entries missing from', () => {
  const out = depcruiseViolations([{ rule: 'no-circular', severity: 'error', from: undefined, to: 'x' }]);
  assert.equal(out.length, 0);
});
