import { runPatternScan, collectRegexViolations } from './regex-engine.mjs';
import { runAstGrepScan } from './ast-engine.mjs';
import { loadSuppressions, isSuppressed, lineHash } from './suppressions.mjs';
import { listSourceFiles } from './enumerate.mjs';
import { printGateReport } from './report.mjs';

/**
 * @param {'file'|'staged'} mode
 * @param {import('./config.mjs').ResolvedConfig} config
 * @returns {{ violations:any[], code:number }}
 */
export function runGate(mode, config) {
  const opts = mode === 'staged' ? { staged: true } : { file: config._fileTarget };
  const files = listSourceFiles(config, opts);
  const scanOpts = opts;
  if (files.length === 0) return { violations: [], code: 0 };

  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);

  let violations = collectRegexViolations(config, runPatternScan(config, scanOpts))
    .filter((v) => allow.has(v.severity));

  const ast = runAstGrepScan(config, files);
  if (!ast.available) process.stderr.write(`⚠ SLOP-GATE: ${ast.errors.join('; ')}\n`);
  for (const e of ast.available ? ast.errors : []) process.stderr.write(`⚠ SLOP-GATE ast-grep: ${e}\n`);
  for (const v of ast.violations) {
    if (allow.has(v.severity)) violations.push({ ...v, lineHash: lineHash(v.fullLine) });
  }

  violations = violations.filter((v) => !isSuppressed(sup.entries, v));

  if (violations.length === 0) return { violations, code: 0 };
  printGateReport(violations, mode);
  return { violations, code: 1 };
}