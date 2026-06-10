// src/checkers/tsc.mjs
/** tsc --noEmit adapter. Always full-project: a staged change can break a non-staged
 *  file and that MUST fail; pre-existing errors are absorbed by the ratchet baseline. */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { localBin, runTool, sourceLine } from './shared.mjs';

export function parseTscOutput(stdout) {
  const errors = [];
  for (const raw of stdout.split('\n')) {
    const m = /^(.+?)\((\d+),(\d+)\): error (TS\d+): (.*)$/.exec(raw);
    if (m) {
      errors.push({ file: m[1].replace(/\\/g, '/'), line: Number(m[2]), code: m[4], message: m[5] });
    } else if (errors.length && /^\s+\S/.test(raw)) {
      errors[errors.length - 1].message += ` ${raw.trim()}`;
    }
  }
  return errors;
}

export default {
  id: 'tsc',
  detect(config, cfg) {
    const tsconfig = join(config.repoRoot, cfg.tsconfig ?? 'tsconfig.json');
    if (!existsSync(tsconfig)) return { available: false, reason: `no ${cfg.tsconfig ?? 'tsconfig.json'}` };
    if (!localBin(config.repoRoot, 'tsc')) return { available: false, reason: 'no local tsc binary' };
    return { available: true };
  },
  run(config, cfg) {
    const tsconfig = join(config.repoRoot, cfg.tsconfig ?? 'tsconfig.json');
    const res = runTool(localBin(config.repoRoot, 'tsc'), ['--noEmit', '--pretty', 'false', '-p', tsconfig], {
      cwd: config.repoRoot, timeout: (cfg.timeout ?? 120) * 1000,
    });
    if (!res.ok) return { violations: [], errors: [`tsc failed: ${res.error}`] };
    const violations = parseTscOutput(res.stdout).map((e) => ({
      id: `tsc-${e.code}`, severity: 'high', category: 'types',
      file: e.file, line: e.line,
      fullLine: sourceLine(config.repoRoot, e.file, e.line),
      text: e.message.trim().slice(0, 90),
      resolution: 'Fix the type error — do not suppress.',
    }));
    return { violations, errors: [] };
  },
};
