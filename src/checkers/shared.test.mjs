// src/checkers/shared.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { localBin, runTool, sourceLine, runToolAsync, runJsonTool, mapLimit } from './shared.mjs';

let failed = 0;
function legacyAssert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const root = mkdtempSync(join(tmpdir(), 'slopgate-shared-'));

// localBin
legacyAssert('missing bin → null', localBin(root, 'tsc') === null);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '#!/bin/sh\necho ok\n');
chmodSync(join(root, 'node_modules/.bin/tsc'), 0o755);
legacyAssert('present bin → path', localBin(root, 'tsc') === join(root, 'node_modules/.bin/tsc'));

// runTool
const ok = runTool('node', ['-e', 'console.log("hi"); process.exit(3)'], { cwd: root, timeout: 10_000 });
legacyAssert('captures stdout', ok.ok === true && ok.stdout.trim() === 'hi');
legacyAssert('captures status', ok.status === 3);
const bad = runTool('/nonexistent-binary-xyz', [], { cwd: root, timeout: 5_000 });
legacyAssert('spawn failure → ok:false + error', bad.ok === false && bad.error.length > 0);
const slow = runTool('node', ['-e', 'setTimeout(()=>{}, 60000)'], { cwd: root, timeout: 300 });
legacyAssert('timeout → ok:false', slow.ok === false);

// sourceLine
writeFileSync(join(root, 'f.ts'), 'line one\nline two\n');
legacyAssert('reads 1-based line', sourceLine(root, 'f.ts', 2) === 'line two');
legacyAssert('out of range → empty', sourceLine(root, 'f.ts', 99) === '');
legacyAssert('missing file → empty', sourceLine(root, 'nope.ts', 1) === '');

rmSync(root, { recursive: true, force: true });

test('legacy shared helpers', () => {
  assert.equal(failed, 0, `${failed} legacy assertion(s) failed`);
});

test('runToolAsync resolves ok:true with stdout for a successful command', async () => {
  const res = await runToolAsync(process.execPath, ['-e', 'process.stdout.write("hi")'], { cwd: process.cwd(), timeout: 5000 });
  assert.equal(res.ok, true);
  assert.equal(res.stdout, 'hi');
  assert.equal(res.status, 0);
});

test('runToolAsync returns ok:false (never throws) for a missing binary', async () => {
  const res = await runToolAsync('definitely-not-a-real-binary-xyz', [], { cwd: process.cwd(), timeout: 5000 });
  assert.equal(res.ok, false);
  assert.ok(res.error);
});

test('runToolAsync resolves ok:false (never throws) when bin is null', async () => {
  const res = await runToolAsync(null, [], { cwd: process.cwd(), timeout: 5000 });
  assert.equal(res.ok, false);
  assert.ok(res.error);
});

test('runJsonTool parses stdout JSON', async () => {
  const r = await runJsonTool('demo', process.execPath, ['-e', 'process.stdout.write(JSON.stringify({a:1}))'], { cwd: process.cwd(), timeout: 5000 });
  assert.deepEqual(r.data, { a: 1 });
  assert.deepEqual(r.errors, []);
});

test('runJsonTool wraps parse errors, returns data:null', async () => {
  const r = await runJsonTool('demo', process.execPath, ['-e', 'process.stdout.write("not json")'], { cwd: process.cwd(), timeout: 5000 });
  assert.equal(r.data, null);
  assert.match(r.errors[0], /demo JSON parse error/);
});

test('mapLimit preserves input order regardless of completion order', async () => {
  const delays = [30, 5, 20, 0];
  const out = await mapLimit(delays, 2, (d, i) => new Promise((res) => setTimeout(() => res(i), d)));
  assert.deepEqual(out, [0, 1, 2, 3]);
});

test('mapLimit caps concurrency', async () => {
  let inFlight = 0; let peak = 0;
  await mapLimit([1,2,3,4,5], 2, async () => {
    inFlight++; peak = Math.max(peak, inFlight);
    await new Promise((r) => setTimeout(r, 10));
    inFlight--;
  });
  assert.ok(peak <= 2, `peak ${peak} exceeded cap 2`);
});
