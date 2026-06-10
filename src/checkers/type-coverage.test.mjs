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
