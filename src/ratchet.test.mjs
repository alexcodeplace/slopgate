// src/ratchet.test.mjs
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fingerprintViolation, loadBaseline, filterNew, writeBaseline } from './ratchet.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const v1 = { engine: 'checker:tsc', id: 'tsc-TS2322', file: 'src/a.ts', text: "Type 'string' is not assignable at 12", fullLine: 'const x: number = s;' };
const v2 = { ...v1, file: 'src/b.ts' };

// fingerprint: stable, digit-normalized, line-text-sensitive
assert('same violation → same fp', fingerprintViolation(v1) === fingerprintViolation({ ...v1 }));
assert('digits in message normalized', fingerprintViolation(v1) === fingerprintViolation({ ...v1, text: "Type 'string' is not assignable at 99" }));
assert('different file → different fp', fingerprintViolation(v1) !== fingerprintViolation(v2));
assert('different line text → different fp', fingerprintViolation(v1) !== fingerprintViolation({ ...v1, fullLine: 'const y: number = s;' }));
assert('fp is 16 hex chars', /^[0-9a-f]{16}$/.test(fingerprintViolation(v1)));

const dir = mkdtempSync(join(tmpdir(), 'slopgate-ratchet-'));
const blPath = join(dir, 'baseline.json');

// missing baseline
const missing = loadBaseline(blPath);
assert('missing → empty entries + missing flag', missing.missing === true && Object.keys(missing.entries).length === 0 && missing.error === null);

// write + load round-trip
const n = writeBaseline(blPath, [v1, v2], '2026-06-10T00:00:00Z');
assert('writeBaseline returns entry count', n === 2);
const loaded = loadBaseline(blPath);
assert('round-trip 2 entries', Object.keys(loaded.entries).length === 2 && loaded.missing === false);
assert('entry carries ruleId+file', loaded.entries[fingerprintViolation(v1)].ruleId === 'tsc-TS2322' && loaded.entries[fingerprintViolation(v1)].file === 'src/a.ts');

// filterNew
const v3 = { ...v1, id: 'tsc-TS7006', text: 'Parameter implicitly any' };
const { fresh, baselinedCount } = filterNew([v1, v2, v3], loaded.entries);
assert('baselined dropped', baselinedCount === 2);
assert('new survives', fresh.length === 1 && fresh[0].id === 'tsc-TS7006');

// malformed
writeFileSync(blPath, '{ not json');
const bad = loadBaseline(blPath);
assert('malformed → empty + error', bad.error !== null && Object.keys(bad.entries).length === 0 && bad.missing === false);
writeFileSync(blPath, JSON.stringify({ version: 1, entries: [] }));
assert('entries-as-array → error', loadBaseline(blPath).error !== null);

rmSync(dir, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
