// src/checkers/jscpd.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import jscpd, { parseJscpdReport, cloneViolations } from './jscpd.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const clones = parseJscpdReport(readFileSync(join(fixDir, 'jscpd.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'jscpd.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(clones) === JSON.stringify(expected));

// staged filtering: only clones touching a staged file produce a violation, pointed at the staged side
const stagedB = cloneViolations(clones, ['src/features/b.ts']);
assert('staged side selected', stagedB.length === 1 && stagedB[0].file === 'src/features/b.ts' && stagedB[0].line === 40);
assert('other side named', stagedB[0].text.includes('src/features/a.ts:10-27'));
const stagedNone = cloneViolations(clones, ['src/other.ts']);
assert('clone not touching staged → dropped', stagedNone.length === 0);
const full = cloneViolations(clones, null);
assert('full mode keeps all, points at first side', full.length === 1 && full[0].file === 'src/features/a.ts');
assert('violation shape', full[0].id === 'jscpd-clone' && full[0].severity === 'high' && full[0].category === 'duplication');

// detect
const root = mkdtempSync(join(tmpdir(), 'slopgate-jscpd-'));
assert('no bin → unavailable', jscpd.detect({ repoRoot: root }, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/jscpd'), '');
assert('bin → available', jscpd.detect({ repoRoot: root }, {}).available === true);
assert('id', jscpd.id === 'jscpd');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
