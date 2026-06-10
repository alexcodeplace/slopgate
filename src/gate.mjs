// src/gate.mjs
import { scanRegex } from './regex-engine.mjs';
import { runAstGrepScan } from './ast-engine.mjs';
import { loadSuppressions, isSuppressed, lineHash } from './suppressions.mjs';
import { listSourceFiles } from './enumerate.mjs';
import { printGateReport } from './report.mjs';
import { loadBaseline, filterNew, stagedRenames } from './ratchet.mjs';
import { CHECKERS } from './checkers/index.mjs';
import { mapLimit, ensureCacheDir } from './checkers/shared.mjs';
import { updateCheckerHealth, isInfraError } from './checkers/health.mjs';
import { join } from 'node:path';

/**
 * Collect raw violations (no suppressions / severity / ratchet filtering).
 * @param {'file'|'staged'|'full'} mode  'full' walks configured roots (baseline snapshot)
 * @param {'fast'|'commit'} tier  checkers run in commit tier only
 * @returns {Promise<{ violations:any[], notices:string[] }>}
 */
export async function collectViolations(mode, config, tier, { fileTarget } = {}) {
  const opts = mode === 'staged' ? { staged: true } : mode === 'file' ? { file: fileTarget } : {};
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
    const eligible = [];
    const outcomes = []; // health: one per enabled checker (infra failures escalate across runs)
    for (const checker of CHECKERS) {
      const cfg = config.checkers[checker.id];
      if (!cfg) continue;
      let det;
      try { det = checker.detect(config, cfg); }
      catch (e) { notices.push(`${checker.id} detect crashed: ${e}`); outcomes.push({ id: checker.id, infraFailed: true, detail: `detect crashed: ${e}` }); continue; }
      if (!det.available) {
        notices.push(`skipped: ${checker.id} (${det.reason})`);
        outcomes.push({ id: checker.id, infraFailed: true, detail: `skipped: ${det.reason}` });
        continue;
      }
      eligible.push({ checker, cfg });
    }
    const started = Date.now();
    const results = await mapLimit(eligible, config.checkerConcurrency ?? 3, async ({ checker, cfg }) => {
      try {
        const res = await checker.run(config, cfg, { files: mode === 'full' ? null : files, mode });
        return { id: checker.id, res };
      } catch (e) {
        return { id: checker.id, res: { violations: [], errors: [`${checker.id} crashed: ${e}`] } };
      }
    });
    const elapsed = (Date.now() - started) / 1000;
    if (elapsed > 30) notices.push(`commit-tier checkers took ${elapsed.toFixed(0)}s (budget ~30s) — check tsc incremental cache / disable slow checkers`);
    for (const { id, res } of results) {
      for (const e of res.errors) notices.push(`${id}: ${e}`);
      outcomes.push({ id, infraFailed: res.errors.some(isInfraError), detail: res.errors.find(isInfraError) });
      for (const v of res.violations) {
        violations.push({ ...v, engine: `checker:${id}`, lineHash: lineHash(v.fullLine ?? '') });
      }
    }
    if (mode === 'staged') { // real commits only — snapshots/tests don't pollute health state
      try {
        const healthPath = join(ensureCacheDir(config), 'checker-health.json');
        notices.push(...updateCheckerHealth(healthPath, outcomes, new Date().toISOString()));
      } catch { /* fail-open */ }
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
 * @param {{ tier?: 'fast'|'commit', fileTarget?: string }} [opts]  default: staged→commit, file→fast
 * @returns {Promise<{ violations:any[], code:number }>}
 */
export async function runGate(mode, config, { tier, fileTarget } = {}) {
  const effTier = tier ?? (mode === 'staged' ? 'commit' : 'fast');
  const { violations: collected, notices } = await collectViolations(mode, config, effTier, { fileTarget });
  for (const n of notices) process.stderr.write(`⚠ SLOPGATE: ${n}\n`);

  let violations = applyGateFilters(collected, config, mode);

  let baselinedCount = 0;
  if (effTier === 'commit') {
    const bl = loadBaseline(config.baselinePath);
    if (bl.error) process.stderr.write(`⚠ SLOPGATE: baseline.json malformed (${bl.error}) — treating as EMPTY (everything is new)\n`);
    if (bl.missing && violations.length) {
      process.stderr.write(`⚠ SLOPGATE: no baseline — run: slopgate baseline --config <config> to absorb pre-existing violations\n`);
    }
    // follow staged renames so a pure file move doesn't re-flag baselined debt
    const renames = mode === 'staged' ? stagedRenames(config.repoRoot) : {};
    ({ fresh: violations, baselinedCount } = filterNew(violations, bl.entries, { renames }));
  }

  if (violations.length === 0) {
    if (baselinedCount > 0) process.stderr.write(`SLOPGATE: clean (${baselinedCount} pre-existing baselined violation(s) ignored)\n`);
    return { violations, code: 0 };
  }
  printGateReport(violations, mode, { baselinedCount });
  return { violations, code: 1 };
}
