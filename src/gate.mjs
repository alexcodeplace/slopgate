// src/gate.mjs
import { runPatternScan, collectRegexViolations } from './regex-engine.mjs';
import { runAstGrepScan } from './ast-engine.mjs';
import { loadSuppressions, isSuppressed, lineHash } from './suppressions.mjs';
import { listSourceFiles } from './enumerate.mjs';
import { printGateReport } from './report.mjs';
import { loadBaseline, filterNew } from './ratchet.mjs';
import { CHECKERS } from './checkers/index.mjs';

/**
 * Collect raw violations (no suppressions / severity / ratchet filtering).
 * @param {'file'|'staged'|'full'} mode  'full' walks configured roots (baseline snapshot)
 * @param {'fast'|'commit'} tier  checkers run in commit tier only
 * @returns {{ violations:any[], notices:string[] }}
 */
export function collectViolations(mode, config, tier) {
  const opts = mode === 'staged' ? { staged: true } : mode === 'file' ? { file: config._fileTarget } : {};
  const files = listSourceFiles(config, opts);
  const notices = [];
  if (files.length === 0 && mode !== 'full') return { violations: [], notices };

  const violations = collectRegexViolations(config, runPatternScan(config, opts));

  const ast = runAstGrepScan(config, mode === 'full' ? null : files);
  if (!ast.available) notices.push(ast.errors.join('; '));
  else for (const e of ast.errors) notices.push(`ast-grep: ${e}`);
  for (const v of ast.violations) {
    if (config.astDisable.has(v.id)) continue;
    violations.push({ ...v, lineHash: lineHash(v.fullLine) });
  }

  if (tier === 'commit') {
    for (const checker of CHECKERS) {
      const cfg = config.checkers[checker.id];
      if (!cfg) continue; // disabled / unconfigured
      const det = checker.detect(config, cfg);
      if (!det.available) { notices.push(`skipped: ${checker.id} (${det.reason})`); continue; }
      const res = checker.run(config, cfg, { files: mode === 'full' ? null : files, mode });
      for (const e of res.errors) notices.push(`${checker.id}: ${e}`);
      for (const v of res.violations) {
        violations.push({ ...v, engine: `checker:${checker.id}`, lineHash: lineHash(v.fullLine ?? '') });
      }
    }
  }
  return { violations, notices };
}

/**
 * @param {'file'|'staged'} mode
 * @param {{ tier?: 'fast'|'commit' }} [opts]  default: staged→commit, file→fast
 * @returns {{ violations:any[], code:number }}
 */
export function runGate(mode, config, { tier } = {}) {
  const effTier = tier ?? (mode === 'staged' ? 'commit' : 'fast');
  const { violations: collected, notices } = collectViolations(mode, config, effTier);
  for (const n of notices) process.stderr.write(`⚠ SLOP-GATE: ${n}\n`);

  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOP-GATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);

  let violations = collected
    .filter((v) => allow.has(v.severity))
    .filter((v) => !isSuppressed(sup.entries, v));

  let baselinedCount = 0;
  if (effTier === 'commit') {
    const bl = loadBaseline(config.baselinePath);
    if (bl.error) process.stderr.write(`⚠ SLOP-GATE: baseline.json malformed (${bl.error}) — treating as EMPTY (everything is new)\n`);
    if (bl.missing && violations.length) {
      process.stderr.write(`⚠ SLOP-GATE: no baseline — run: slop-gate baseline --config <config> to absorb pre-existing violations\n`);
    }
    ({ fresh: violations, baselinedCount } = filterNew(violations, bl.entries));
  }

  if (violations.length === 0) {
    if (baselinedCount > 0) process.stderr.write(`SLOP-GATE: clean (${baselinedCount} pre-existing baselined violation(s) ignored)\n`);
    return { violations, code: 0 };
  }
  printGateReport(violations, mode, { baselinedCount });
  return { violations, code: 1 };
}
