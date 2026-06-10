// src/checkers/depcruise.test.mjs
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import depcruise, { parseDepcruiseOutput, depcruiseViolations } from './depcruise.mjs';

let failed = 0;
function assert(label, ok) { console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`); if (!ok) failed++; }

const here = dirname(fileURLToPath(import.meta.url));
const fixDir = join(here, '../../rules/baseline/fixtures/checker-outputs');
const parsed = parseDepcruiseOutput(readFileSync(join(fixDir, 'depcruise.json'), 'utf8'));
const expected = JSON.parse(readFileSync(join(fixDir, 'depcruise.expected.json'), 'utf8'));
assert('fixture parses to expected', JSON.stringify(parsed) === JSON.stringify(expected));

const vios = depcruiseViolations(parsed);
assert('error → critical', vios[0].severity === 'critical' && vios[0].id === 'depcruise-no-circular');
assert('warn → high', vios[1].severity === 'high');
assert('info dropped', vios.length === 2);
assert('edge named in text', vios[0].text.includes('src/a.ts → src/b.ts'));
assert('category architecture', vios[0].category === 'architecture' && vios[0].file === 'src/a.ts' && vios[0].line === 1);

// detect: needs bin + a rules file
const root = mkdtempSync(join(tmpdir(), 'slopgate-dc-'));
const config = { repoRoot: root, configDir: join(root, '.slopgate') };
mkdirSync(config.configDir, { recursive: true });
assert('no bin → unavailable', depcruise.detect(config, {}).available === false);
mkdirSync(join(root, 'node_modules/.bin'), { recursive: true });
writeFileSync(join(root, 'node_modules/.bin/depcruise'), '');
assert('bin but no rules → unavailable', depcruise.detect(config, {}).available === false);
writeFileSync(join(config.configDir, 'depcruise.cjs'), 'module.exports={};');
assert('slopgate rules file → available', depcruise.detect(config, {}).available === true);
assert('id', depcruise.id === 'depcruise');

rmSync(root, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
