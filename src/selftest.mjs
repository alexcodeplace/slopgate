import { readFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runAstGrepScan } from './ast-engine.mjs';
import { parseTscOutput } from './checkers/tsc.mjs';
import { parseKnipOutput } from './checkers/knip.mjs';
import { parseJscpdReport } from './checkers/jscpd.mjs';
import { parseDepcruiseOutput } from './checkers/depcruise.mjs';
import { parseTypeCoverageOutput } from './checkers/type-coverage.mjs';

/** @param {import('./config.mjs').ResolvedConfig} config */
export function runSelfTest(config) {
  let failed = 0;
  for (const p of config.patterns) {
    if (!p.canary) { console.error(`WARN ${p.id}: no canary — cannot prove rule still fires`); continue; }
    let re;
    try { re = new RegExp(p.pattern, p.flags || undefined); } catch (e) { console.error(`FAIL ${p.id}: regex invalid: ${e}`); failed++; continue; }
    if (!re.test(p.canary)) { console.error(`FAIL ${p.id}: canary not matched: ${p.canary}`); failed++; }
    else console.error(`OK ${p.id}`);
    for (const neg of [].concat(p.negativeCanary ?? [])) {
      if (re.test(neg)) { console.error(`FAIL ${p.id}: negative canary matched: ${neg}`); failed++; }
      else console.error(`OK ${p.id} (negative)`);
    }
  }
  const ast = runAstGrepScan(config, config.fixturesDirs, { rawTargets: true });
  if (!ast.available) {
    console.error(`WARN ast-grep unavailable — bucket-B self-test skipped: ${ast.errors.join('; ')}`);
  } else if (!ast.violations.some((v) => v.id === 'slopgate-canary')) {
    if (ast.errors?.length) for (const e of ast.errors) console.error(`FAIL ast: ${e}`);
    console.error('FAIL ast-grep canary: slopgate-canary did not fire on fixtures'); failed++;
  } else {
    console.error(`OK ast-grep canary (${ast.violations.length} fixture violations)`);
  }
  // checker parser fixtures: recorded real tool outputs must parse to expected shapes.
  // Catches tool-output-format drift without invoking the tools.
  const fixDir = join(dirname(fileURLToPath(import.meta.url)), '../rules/baseline/fixtures/checker-outputs');
  const PARSER_FIXTURES = [
    { id: 'tsc', input: 'tsc.txt', expected: 'tsc.expected.json', parse: (t) => parseTscOutput(t) },
    { id: 'knip', input: 'knip.json', expected: 'knip.expected.json', parse: (t) => parseKnipOutput(t) },
    { id: 'jscpd', input: 'jscpd.json', expected: 'jscpd.expected.json', parse: (t) => parseJscpdReport(t) },
    { id: 'depcruise', input: 'depcruise.json', expected: 'depcruise.expected.json', parse: (t) => parseDepcruiseOutput(t) },
    { id: 'type-coverage', input: 'type-coverage.txt', expected: 'type-coverage.expected.json', parse: (t) => parseTypeCoverageOutput(t, '/repo') },
  ];
  for (const f of PARSER_FIXTURES) {
    const inPath = join(fixDir, f.input);
    const expPath = join(fixDir, f.expected);
    if (!existsSync(inPath) || !existsSync(expPath)) { console.error(`FAIL parser ${f.id}: fixture missing`); failed++; continue; }
    try {
      const got = JSON.stringify(f.parse(readFileSync(inPath, 'utf8')));
      const want = JSON.stringify(JSON.parse(readFileSync(expPath, 'utf8')));
      if (got !== want) { console.error(`FAIL parser ${f.id}: parsed output != expected`); failed++; }
      else console.error(`OK parser ${f.id}`);
    } catch (e) { console.error(`FAIL parser ${f.id}: ${e}`); failed++; }
  }
  return failed ? 1 : 0;
}