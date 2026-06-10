import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runAstGrepScan } from './ast-engine.mjs';
import { compileLineRegex } from './regex-engine.mjs';
import { parseTscOutput } from './checkers/tsc.mjs';
import { parseKnipOutput } from './checkers/knip.mjs';
import { parseJscpdReport } from './checkers/jscpd.mjs';
import { parseDepcruiseOutput } from './checkers/depcruise.mjs';
import { parseTypeCoverageOutput } from './checkers/type-coverage.mjs';
import { BASELINE_AST_DIR } from '../rules/baseline/index.mjs';

/** @param {import('./config.mjs').ResolvedConfig} config */
export function runSelfTest(config) {
  let failed = 0;
  for (const p of config.patterns) {
    if (!p.canary) { console.error(`WARN ${p.id}: no canary — cannot prove rule still fires`); continue; }
    let re;
    try { re = compileLineRegex(p.pattern, p.flags); } catch (e) { console.error(`FAIL ${p.id}: regex invalid: ${e}`); failed++; continue; }
    if (!re.test(p.canary)) { console.error(`FAIL ${p.id}: canary not matched: ${p.canary}`); failed++; }
    else console.error(`OK ${p.id}`);
    for (const neg of [].concat(p.negativeCanary ?? [])) {
      if (re.test(neg)) { console.error(`FAIL ${p.id}: negative canary matched: ${neg}`); failed++; }
      else console.error(`OK ${p.id} (negative)`);
    }
  }
  // config path sanity: dangling roots/fixtures dirs = silent-zero-results rot (zync F1 class)
  for (const r of config.roots) {
    if (!existsSync(r)) { console.error(`FAIL config: root missing: ${r}`); failed++; }
  }
  const fixturesDirs = [];
  for (const d of config.fixturesDirs) {
    if (!existsSync(d)) { console.error(`FAIL config: fixtures dir missing: ${d}`); failed++; }
    else fixturesDirs.push(d);
  }
  const ast = runAstGrepScan(config, fixturesDirs, { rawTargets: true });
  if (!ast.available) {
    console.error(`WARN ast-grep unavailable — bucket-B self-test skipped: ${ast.errors.join('; ')}`);
  } else if (!ast.violations.some((v) => v.id === 'slopgate-canary')) {
    if (ast.errors?.length) for (const e of ast.errors) console.error(`FAIL ast: ${e}`);
    console.error('FAIL ast-grep canary: slopgate-canary did not fire on fixtures'); failed++;
  } else {
    console.error(`OK ast-grep canary (${ast.violations.length} fixture violations)`);
  }
  // project ast rules: every rule yml must fire at least once on the fixtures scan.
  const projectAstDirs = (config.astRuleDirs || []).filter((d) => d !== BASELINE_AST_DIR && existsSync(d));
  if (!ast.available) {
    if (projectAstDirs.length) console.error('WARN ast-grep unavailable — project ast rules not verified');
  } else {
    for (const dir of projectAstDirs) {
      for (const f of readdirSync(dir).filter((n) => n.endsWith('.yml') || n.endsWith('.yaml'))) {
        const m = /^id:\s*(\S+)/m.exec(readFileSync(join(dir, f), 'utf8'));
        if (!m) { console.error(`FAIL ast ${f}: no "id:" line`); failed++; continue; }
        const id = m[1];
        if (config.astDisable.has(id)) { console.error(`SKIP ast ${id} (astDisable)`); continue; }
        if (!ast.violations.some((v) => v.id === id)) {
          console.error(`FAIL ast ${id}: did not fire on fixtures — add a trigger to the project fixtures dir`); failed++;
        } else {
          console.error(`OK ast ${id}`);
        }
      }
    }
  }
  // checker parser fixtures: recorded real tool outputs must parse to expected shapes.
  // Catches tool-output-format drift without invoking the tools.
  const fixDir = join(dirname(fileURLToPath(import.meta.url)), '../rules/baseline/fixtures/checker-outputs');
  const PARSER_FIXTURES = [
    { id: 'tsc', input: 'tsc.txt', expected: 'tsc.expected.json', parse: (t) => parseTscOutput(t) },
    { id: 'knip', input: 'knip.json', expected: 'knip.expected.json', parse: (t) => parseKnipOutput(JSON.parse(t)) },
    { id: 'jscpd', input: 'jscpd.json', expected: 'jscpd.expected.json', parse: (t) => parseJscpdReport(t) },
    { id: 'depcruise', input: 'depcruise.json', expected: 'depcruise.expected.json', parse: (t) => parseDepcruiseOutput(JSON.parse(t)) },
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