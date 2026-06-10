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
