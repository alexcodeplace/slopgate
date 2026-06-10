// src/selftest.test.mjs
// Proves runSelfTest catches: non-firing project ast rules, dangling fixtures dirs.
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { resolveConfig } from './config.mjs';
import { runSelfTest } from './selftest.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const haveAstGrep = spawnSync('ast-grep', ['--version'], { encoding: 'utf8' }).status === 0;
if (!haveAstGrep) {
  console.log('SKIP: ast-grep not on PATH — project-ast self-test assertions not verifiable here');
  process.exit(0);
}

const root = mkdtempSync(join(tmpdir(), 'slopgate-selftest-'));
const sg = join(root, '.slop-gate');
mkdirSync(join(sg, 'rules/ast'), { recursive: true });
mkdirSync(join(sg, 'fixtures/src'), { recursive: true });
mkdirSync(join(root, 'src'), { recursive: true });

writeFileSync(join(sg, 'config.mjs'),
  "export default { roots: ['src'], rules: [], astRules: './rules/ast', fixtures: './fixtures' };\n");
writeFileSync(join(sg, 'rules/ast/test-fire.yml'), [
  'id: test-fire', 'language: tsx', 'severity: error', 'message: test',
  'rule:', '  pattern: dangerouslyNeverWrite($$$)', '',
].join('\n'));
writeFileSync(join(sg, 'fixtures/src/canary.tsx'), 'dangerouslyNeverWrite(1);\n');

const cfg1 = await resolveConfig(join(sg, 'config.mjs'));
assert('firing project ast rule → exit 0', runSelfTest(cfg1) === 0);

writeFileSync(join(sg, 'rules/ast/never-fires.yml'), [
  'id: never-fires', 'language: tsx', 'severity: error', 'message: test',
  'rule:', '  pattern: zzzNeverCalledFn($$$)', '',
].join('\n'));
const cfg2 = await resolveConfig(join(sg, 'config.mjs'));
assert('non-firing project ast rule → exit 1', runSelfTest(cfg2) === 1);

rmSync(join(sg, 'rules/ast/never-fires.yml'));
rmSync(join(sg, 'fixtures'), { recursive: true, force: true });
writeFileSync(join(sg, 'config.mjs'),
  "export default { roots: ['src'], rules: [], fixtures: './fixtures' };\n");
const cfg3 = await resolveConfig(join(sg, 'config.mjs'));
assert('declared fixtures dir missing → exit 1 (isolated)', runSelfTest(cfg3) === 1);

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
