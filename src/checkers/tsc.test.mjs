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
