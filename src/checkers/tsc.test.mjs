// src/checkers/tsc.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
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
const pathHasTsc = spawnSync('tsc', ['--version'], { encoding: 'utf8' }).status === 0;

const root = mkdtempSync(join(tmpdir(), 'slopgate-tsc-'));
assert('no tsconfig → unavailable', tsc.detect({ repoRoot: root }, {}).available === false);
writeFileSync(join(root, 'tsconfig.json'), '{}');
assert('no local tsc → PATH fallback decides', tsc.detect({ repoRoot: root }, {}).available === pathHasTsc);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '');
assert('tsconfig + local bin → available', tsc.detect({ repoRoot: root }, {}).available === true);
assert('custom tsconfig honored', tsc.detect({ repoRoot: root }, { tsconfig: 'tsconfig.app.json' }).available === false);

// array form (spec 3.4: monorepo support)
writeFileSync(join(root, 'tsconfig.app.json'), '{}');
assert('array: all exist → available',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'tsconfig.app.json'] }).available === true);
assert('array: one missing → unavailable',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'nope.json'] }).available === false);
assert('array: missing reason names the file',
  tsc.detect({ repoRoot: root }, { tsconfig: ['tsconfig.json', 'nope.json'] }).reason === 'no nope.json');
assert('id', tsc.id === 'tsc');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
