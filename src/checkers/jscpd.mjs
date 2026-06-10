// src/checkers/jscpd.mjs
/** jscpd adapter — copy-paste clones ("reimplemented instead of imported"). Scans the
 *  configured roots; in staged mode a clone counts only if one side overlaps a staged
 *  file (violation points at the staged side, excerpt names the other). */
import { readFileSync, rmSync, mkdtempSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseJscpdReport(jsonText) {
  const j = JSON.parse(jsonText);
  return (j.duplicates ?? []).map((d) => ({
    firstFile: d.firstFile.name,
    firstStart: d.firstFile.start ?? d.firstFile.startLoc?.line ?? 1,
    firstEnd: d.firstFile.end ?? d.firstFile.endLoc?.line ?? 1,
    secondFile: d.secondFile.name,
    secondStart: d.secondFile.start ?? d.secondFile.startLoc?.line ?? 1,
    secondEnd: d.secondFile.end ?? d.secondFile.endLoc?.line ?? 1,
    lines: d.lines,
  }));
}

/** @param {string[]|null} stagedFiles null = full mode (keep every clone, point at first side) */
export function cloneViolations(clones, stagedFiles, repoRoot = null) {
  const staged = stagedFiles ? new Set(stagedFiles) : null;
  const out = [];
  for (const c of clones) {
    let mine; let other; let line;
    if (!staged) {
      mine = c.firstFile; other = `${c.secondFile}:${c.secondStart}-${c.secondEnd}`; line = c.firstStart;
    } else if (staged.has(c.firstFile)) {
      mine = c.firstFile; other = `${c.secondFile}:${c.secondStart}-${c.secondEnd}`; line = c.firstStart;
    } else if (staged.has(c.secondFile)) {
      mine = c.secondFile; other = `${c.firstFile}:${c.firstStart}-${c.firstEnd}`; line = c.secondStart;
    } else continue;
    out.push({
      id: 'jscpd-clone', severity: 'high', category: 'duplication',
      file: mine, line,
      fullLine: repoRoot ? sourceLine(repoRoot, mine, line) : '',
      text: `duplicates ${other} (${c.lines} lines)`.slice(0, 90),
      resolution: 'Extract a shared util / import the existing implementation.',
    });
  }
  return out;
}

export default {
  id: 'jscpd',
  detect(config) {
    if (!localBin(config.repoRoot, 'jscpd')) return { available: false, reason: 'no local jscpd binary' };
    return { available: true };
  },
  run(config, cfg, { files = null } = {}) {
    const outDir = mkdtempSync(join(tmpdir(), 'slopgate-jscpd-'));
    const res = runTool(localBin(config.repoRoot, 'jscpd'), [
      ...config.rootsRel,
      '--min-tokens', String(cfg.minTokens ?? 50),
      '--reporters', 'json', '--output', outDir, '--silent',
    ], { cwd: config.repoRoot, timeout: (cfg.timeout ?? 60) * 1000 });
    try {
      if (!res.ok) return { violations: [], errors: [`jscpd failed: ${res.error}`] };
      const reportPath = join(outDir, 'jscpd-report.json');
      if (!existsSync(reportPath)) return { violations: [], errors: ['jscpd produced no report'] };
      let clones;
      try { clones = parseJscpdReport(readFileSync(reportPath, 'utf8')); }
      catch (e) { return { violations: [], errors: [`jscpd JSON parse error: ${e}`] }; }
      return { violations: cloneViolations(clones, files, config.repoRoot), errors: [] };
    } finally {
      rmSync(outDir, { recursive: true, force: true });
    }
  },
};
