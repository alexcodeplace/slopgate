// src/checkers/depcruise.mjs
/** dependency-cruiser adapter — the architecture gate: layer boundaries, cycles,
 *  orphans, encoded as rules in .slopgate/depcruise.cjs (project-pinned). */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runJsonTool } from './shared.mjs';

const SEVERITY_MAP = { error: 'critical', warn: 'high' }; // info → dropped

export function parseDepcruiseOutput(j) {
  return (j.summary?.violations ?? []).map((v) => ({
    rule: v.rule?.name ?? 'unknown', severity: v.rule?.severity ?? 'error', from: v.from, to: v.to,
  }));
}

export function depcruiseViolations(parsed) {
  const out = [];
  for (const v of parsed) {
    const severity = SEVERITY_MAP[v.severity];
    if (!severity) continue;
    if (!v.from) continue;
    out.push({
      id: `depcruise-${v.rule}`, severity, category: 'architecture',
      file: v.from, line: 1, fullLine: '',
      text: `${v.from} → ${v.to} violates ${v.rule}`.slice(0, 90),
      resolution: 'Respect the dependency rule — restructure the import, do not relax the rule.',
    });
  }
  return out;
}

function rulesFile(config, cfg) {
  const candidates = [
    cfg.rules ? join(config.configDir, cfg.rules) : null,
    join(config.configDir, 'depcruise.cjs'),
    join(config.repoRoot, '.dependency-cruiser.js'),
    join(config.repoRoot, '.dependency-cruiser.cjs'),
    join(config.repoRoot, '.dependency-cruiser.json'),
  ].filter(Boolean);
  return candidates.find(existsSync) ?? null;
}

export default {
  id: 'depcruise',
  detect(config, cfg) {
    if (!localBin(config.repoRoot, 'depcruise')) return { available: false, reason: 'no local depcruise binary' };
    if (!rulesFile(config, cfg)) return { available: false, reason: 'no depcruise rules file' };
    return { available: true };
  },
  async run(config, cfg) {
    const { data, errors } = await runJsonTool('depcruise', localBin(config.repoRoot, 'depcruise'),
      ['--config', rulesFile(config, cfg), '--output-type', 'json', ...config.rootsRel],
      { cwd: config.repoRoot, timeout: (cfg.timeout ?? 60) * 1000 });
    if (data == null) return { violations: [], errors };
    return { violations: depcruiseViolations(parseDepcruiseOutput(data)), errors };
  },
};
