// src/gate.tier.test.mjs
// Uses a fake checker injected via the registry seam to prove tier + ratchet behavior
// without external tools. Regex/ast paths already covered by self-test.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { resolveConfig } from './config.mjs';
import { collectViolations, runGate } from './gate.mjs';
import { CHECKERS } from './checkers/index.mjs';
import { writeBaseline } from './ratchet.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

assert('registry has 6 checkers', CHECKERS.length === 6
  && JSON.stringify(CHECKERS.map((c) => c.id)) === JSON.stringify(['tsc', 'knip', 'jscpd', 'depcruise', 'type-coverage', 'diff-shape']));
assert('every checker has detect+run', CHECKERS.every((c) => typeof c.detect === 'function' && typeof c.run === 'function'));

const repo = mkdtempSync(join(tmpdir(), 'slopgate-gate-'));
execSync('git init -q', { cwd: repo });
mkdirSync(join(repo, '.slop-gate'), { recursive: true });
mkdirSync(join(repo, 'src'), { recursive: true });
writeFileSync(join(repo, 'src/a.ts'), '// placeholder for now\nconst ok = 1;\n');
writeFileSync(join(repo, '.slop-gate/config.mjs'), `export default {
  roots: ['src'],
  baseline: ['no-stubs'],
  checkers: { 'fake': true },
};\n`);
execSync('git add src/a.ts', { cwd: repo });

const fake = {
  id: 'fake',
  detect: () => ({ available: true }),
  run: () => ({ violations: [{ id: 'fake-finding', severity: 'high', category: 'test', file: 'src/a.ts', line: 2, fullLine: 'const ok = 1;', text: 'fake', resolution: 'n/a' }], errors: [] }),
};
CHECKERS.push(fake);

const config = await resolveConfig(join(repo, '.slop-gate/config.mjs'));

// fast tier: regex fires, checker does NOT
const fast = collectViolations('staged', config, 'fast');
assert('fast tier: regex violation present', fast.violations.some((v) => v.id === 'no-stubs-placeholder'));
assert('fast tier: checker not run', !fast.violations.some((v) => v.id === 'fake-finding'));

// commit tier: checker runs, engine tagged, lineHash attached
const commit = collectViolations('staged', config, 'commit');
const fakeV = commit.violations.find((v) => v.id === 'fake-finding');
assert('commit tier: checker violation present', !!fakeV);
assert('checker violation engine-tagged', fakeV.engine === 'checker:fake');
assert('checker violation has lineHash', typeof fakeV.lineHash === 'string' && fakeV.lineHash.length === 40);

// disabled checker never runs even if registered
const noCfg = { ...config, checkers: {} };
const none = collectViolations('staged', noCfg, 'commit');
assert('unconfigured checker skipped silently', !none.violations.some((v) => v.id === 'fake-finding'));

// runGate commit tier blocks (exit 1), fast staged ignores baseline
const gateRes = runGate('staged', config);
assert('staged default = commit tier, blocks', gateRes.code === 1);

// baseline absorbs → exit 0
writeBaseline(config.baselinePath, gateRes.violations, '2026-06-10T00:00:00Z');
const after = runGate('staged', config);
assert('all baselined → exit 0', after.code === 0 && after.violations.length === 0);

// new violation on top of baseline → exit 1 with only the new one
writeFileSync(join(repo, 'src/a.ts'), '// placeholder for now\nconst ok = 1;\n// TODO: implement later\n');
execSync('git add src/a.ts', { cwd: repo });
const fresh = runGate('staged', config);
assert('only NEW violation fails', fresh.code === 1 && fresh.violations.length === 1 && fresh.violations[0].line === 3 && fresh.violations[0].id === 'no-stubs-placeholder');
assert('baselined ones still absorbed', !fresh.violations.some((v) => v.line === 1 || v.id === 'fake-finding'));

CHECKERS.pop();
rmSync(repo, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
