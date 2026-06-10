// src/config.checkers.test.mjs
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { execSync } from 'node:child_process';
import { resolveConfig } from './config.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const repo = mkdtempSync(join(tmpdir(), 'slopgate-cfg-'));
execSync('git init -q', { cwd: repo });
mkdirSync(join(repo, '.slopgate'), { recursive: true });
mkdirSync(join(repo, 'src'), { recursive: true });
writeFileSync(join(repo, '.slopgate/config.mjs'), `export default {
  roots: ['src'],
  astDisable: ['console-debug-left'],
  checkers: {
    tsc: true,
    knip: false,
    jscpd: { minTokens: 70 },
  },
};\n`);

const cfg = await resolveConfig(join(repo, '.slopgate/config.mjs'));
assert('true normalizes to {}', JSON.stringify(cfg.checkers.tsc) === '{}');
assert('false drops the checker', !('knip' in cfg.checkers));
assert('object passes through', cfg.checkers.jscpd.minTokens === 70);
assert('absent checker absent', !('depcruise' in cfg.checkers));
assert('astDisable is a Set', cfg.astDisable instanceof Set && cfg.astDisable.has('console-debug-left'));
assert('baselinePath under configDir', cfg.baselinePath === join(repo, '.slopgate/baseline.json'));

// defaults when keys absent (separate config file — ESM import cache keys by path)
writeFileSync(join(repo, '.slopgate/config2.mjs'), 'export default { roots: ["src"] };\n');
const cfg2 = await resolveConfig(join(repo, '.slopgate/config2.mjs'));
assert('no checkers key → empty object', Object.keys(cfg2.checkers).length === 0);
assert('no astDisable → empty Set', cfg2.astDisable instanceof Set && cfg2.astDisable.size === 0);

rmSync(repo, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
