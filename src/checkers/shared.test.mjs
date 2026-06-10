// src/checkers/shared.test.mjs
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { localBin, runTool, sourceLine } from './shared.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const root = mkdtempSync(join(tmpdir(), 'slopgate-shared-'));

// localBin
assert('missing bin → null', localBin(root, 'tsc') === null);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/tsc'), '#!/bin/sh\necho ok\n');
chmodSync(join(root, 'node_modules/.bin/tsc'), 0o755);
assert('present bin → path', localBin(root, 'tsc') === join(root, 'node_modules/.bin/tsc'));

// runTool
const ok = runTool('node', ['-e', 'console.log("hi"); process.exit(3)'], { cwd: root, timeout: 10_000 });
assert('captures stdout', ok.ok === true && ok.stdout.trim() === 'hi');
assert('captures status', ok.status === 3);
const bad = runTool('/nonexistent-binary-xyz', [], { cwd: root, timeout: 5_000 });
assert('spawn failure → ok:false + error', bad.ok === false && bad.error.length > 0);
const slow = runTool('node', ['-e', 'setTimeout(()=>{}, 60000)'], { cwd: root, timeout: 300 });
assert('timeout → ok:false', slow.ok === false);

// sourceLine
writeFileSync(join(root, 'f.ts'), 'line one\nline two\n');
assert('reads 1-based line', sourceLine(root, 'f.ts', 2) === 'line two');
assert('out of range → empty', sourceLine(root, 'f.ts', 99) === '');
assert('missing file → empty', sourceLine(root, 'nope.ts', 1) === '');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
