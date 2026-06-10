// src/gate.mjs
import { scanRegex } from './regex-engine.mjs';
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

  const violations = scanRegex(config, files, { fileMode: mode === 'file' });

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
 * Severity-allow + suppression filter shared by the gate and the baseline snapshot.
 * Emits the malformed-suppressions warning once. Does NOT apply ratchet/baseline.
 * @param {any[]} violations
 * @param {'file'|'staged'} mode  selects the gate.<mode> severity allow-list
 * @returns {any[]}
 */
export function applyGateFilters(violations, config, mode) {
  const allow = new Set(config.gate[mode] ?? ['critical', 'high']);
  const sup = loadSuppressions(config.suppressionsPath);
  if (sup.error) process.stderr.write(`⚠ SLOPGATE: suppressions.json malformed (${sup.error}) — treating as EMPTY\n`);
  return violations
    .filter((v) => allow.has(v.severity))
    .filter((v) => !isSuppressed(sup.entries, v));
}

/**
 * @param {'file'|'staged'} mode
 * @param {{ tier?: 'fast'|'commit' }} [opts]  default: staged→commit, file→fast
 * @returns {{ violations:any[], code:number }}
 */
export function runGate(mode, config, { tier } = {}) {
  const effTier = tier ?? (mode === 'staged' ? 'commit' : 'fast');
  const { violations: collected, notices } = collectViolations(mode, config, effTier);
  for (const n of notices) process.stderr.write(`⚠ SLOPGATE: ${n}\n`);

  let violations = applyGateFilters(collected, config, mode);

  let baselinedCount = 0;
  if (effTier === 'commit') {
    const bl = loadBaseline(config.baselinePath);
    if (bl.error) process.stderr.write(`⚠ SLOPGATE: baseline.json malformed (${bl.error}) — treating as EMPTY (everything is new)\n`);
    if (bl.missing && violations.length) {
      process.stderr.write(`⚠ SLOPGATE: no baseline — run: slopgate baseline --config <config> to absorb pre-existing violations\n`);
    }
    ({ fresh: violations, baselinedCount } = filterNew(violations, bl.entries));
  }

  if (violations.length === 0) {
    if (baselinedCount > 0) process.stderr.write(`SLOPGATE: clean (${baselinedCount} pre-existing baselined violation(s) ignored)\n`);
    return { violations, code: 0 };
  }
  printGateReport(violations, mode, { baselinedCount });
  return { violations, code: 1 };
}
