// src/checkers/knip.mjs
/** knip adapter — unused files/exports/types/deps. Full-repo by nature (dead code is a
 *  whole-graph property); pre-existing findings are absorbed by the ratchet baseline.
 *  Requires explicit knip config — knip without config is too noisy to gate on. */
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runJsonTool, sourceLine } from './shared.mjs';

const ISSUE_TYPES = ['dependencies', 'devDependencies', 'unlisted', 'exports', 'types', 'duplicates'];

const RESOLUTIONS = {
  files: 'Delete the unused file (or wire it in if it was meant to be used).',
  exports: 'Remove the unused export (or its consumer was deleted by mistake).',
  types: 'Remove the unused exported type.',
  dependencies: 'Uninstall the unused dependency.',
  devDependencies: 'Uninstall the unused devDependency.',
  unlisted: 'Add the dependency to package.json (it is imported but unlisted).',
  duplicates: 'Deduplicate the export.',
};

export function parseKnipOutput(j) {
  const out = [];
  for (const f of j.files ?? []) out.push({ type: 'files', file: f, line: 1, name: f });
  for (const issue of j.issues ?? []) {
    for (const type of ISSUE_TYPES) {
      for (const item of issue[type] ?? []) {
        out.push({ type, file: issue.file, line: item.line ?? 1, name: item.name ?? String(item) });
      }
    }
  }
  return out;
}

function hasKnipConfig(repoRoot) {
  if (['knip.json', 'knip.jsonc', 'knip.config.ts', 'knip.config.js'].some((f) => existsSync(join(repoRoot, f)))) return true;
  try { return 'knip' in JSON.parse(readFileSync(join(repoRoot, 'package.json'), 'utf8')); }
  catch { return false; }
}

export default {
  id: 'knip',
  detect(config) {
    if (!localBin(config.repoRoot, 'knip')) return { available: false, reason: 'no local knip binary' };
    if (!hasKnipConfig(config.repoRoot)) return { available: false, reason: 'no knip config' };
    return { available: true };
  },
  async run(config, cfg) {
    const bin = localBin(config.repoRoot, 'knip');
    if (!bin) return { violations: [], errors: ['knip failed: no local knip binary'] };
    const { data, errors } = await runJsonTool('knip', bin,
      ['--reporter', 'json', '--no-exit-code'], { cwd: config.repoRoot, timeout: (cfg.timeout ?? 90) * 1000 });
    if (data == null) return { violations: [], errors };
    const findings = parseKnipOutput(data);
    const violations = findings.map((f) => ({
      id: `knip-${f.type}`, severity: 'high', category: 'dead-code',
      file: f.file, line: f.line,
      fullLine: f.type === 'files' ? '' : sourceLine(config.repoRoot, f.file, f.line),
      text: `unused ${f.type === 'files' ? 'file' : f.type}: ${f.name}`.slice(0, 90),
      resolution: RESOLUTIONS[f.type],
    }));
    return { violations, errors };
  },
};
