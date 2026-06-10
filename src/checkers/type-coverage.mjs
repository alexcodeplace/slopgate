// src/checkers/type-coverage.mjs
/** type-coverage adapter — every implicitly-any expression is a violation; the ratchet
 *  baseline absorbs pre-existing ones, so coverage can only rise. No percent watermark:
 *  fingerprints give per-expression precision a percentage can't. */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseTypeCoverageOutput(stdout, repoRoot) {
  const out = [];
  for (const raw of stdout.split('\n')) {
    const m = /^(.+?\.(?:ts|tsx|mts|cts)):(\d+):(\d+):? (.*)$/.exec(raw.trim());
    if (!m) continue;
    let file = m[1].replace(/\\/g, '/');
    if (repoRoot && file.startsWith(`${repoRoot}/`)) file = file.slice(repoRoot.length + 1);
    out.push({ file, line: Number(m[2]), name: m[4] });
  }
  return out;
}

export default {
  id: 'type-coverage',
  detect(config) {
    if (!existsSync(join(config.repoRoot, 'tsconfig.json'))) return { available: false, reason: 'no tsconfig.json' };
    if (!localBin(config.repoRoot, 'type-coverage')) return { available: false, reason: 'no local type-coverage binary' };
    return { available: true };
  },
  run(config, cfg) {
    const res = runTool(localBin(config.repoRoot, 'type-coverage'), ['--detail'], {
      cwd: config.repoRoot, timeout: (cfg.timeout ?? 120) * 1000,
    });
    if (!res.ok) return { violations: [], errors: [`type-coverage failed: ${res.error}`] };
    const violations = parseTypeCoverageOutput(res.stdout, config.repoRoot).map((e) => ({
      id: 'type-coverage-uncovered', severity: 'high', category: 'types',
      file: e.file, line: e.line,
      fullLine: sourceLine(config.repoRoot, e.file, e.line),
      text: `implicitly any: ${e.name}`.slice(0, 90),
      resolution: 'Type this expression precisely.',
    }));
    return { violations, errors: [] };
  },
};
