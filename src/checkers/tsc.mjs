// src/checkers/tsc.mjs
/** tsc --noEmit adapter. Always full-project: a staged change can break a non-staged
 *  file and that MUST fail; pre-existing errors are absorbed by the ratchet baseline.
 *  cfg.tsconfig: string | string[] — monorepos list one tsconfig per package/app.
 *  Binary: local node_modules/.bin/tsc preferred, PATH tsc fallback (ast-grep precedent). */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { localBin, runToolAsync, sourceLine } from './shared.mjs';

export function resolveTscBin(repoRoot) {
  const local = localBin(repoRoot, 'tsc');
  if (local) return { bin: local, source: 'local' };
  const probe = spawnSync('tsc', ['--version'], { encoding: 'utf8' });
  if (probe.status === 0) return { bin: 'tsc', source: 'path' };
  return null;
}

function tsconfigList(cfg) {
  return [].concat(cfg.tsconfig ?? 'tsconfig.json');
}

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
    for (const rel of tsconfigList(cfg)) {
      if (!existsSync(join(config.repoRoot, rel))) return { available: false, reason: `no ${rel}` };
    }
    if (!resolveTscBin(config.repoRoot)) return { available: false, reason: 'no tsc binary (local or PATH)' };
    return { available: true };
  },
  async run(config, cfg) {
    const resolved = resolveTscBin(config.repoRoot);
    const bin = resolved.bin;
    const violations = [];
    const errors = [];
    if (resolved.source === 'path') errors.push('tsc: using PATH binary (version not pinned — results may differ from CI)');
    for (const rel of tsconfigList(cfg)) {
      const res = await runToolAsync(bin, ['--noEmit', '--pretty', 'false', '-p', join(config.repoRoot, rel)], {
        cwd: config.repoRoot, timeout: (cfg.timeout ?? 120) * 1000,
      });
      if (!res.ok && res.status == null) { errors.push(`tsc(${rel}) failed: ${res.error}`); continue; }
      violations.push(...parseTscOutput(res.stdout).map((e) => ({
        id: `tsc-${e.code}`, severity: 'high', category: 'types',
        file: e.file, line: e.line,
        fullLine: sourceLine(config.repoRoot, e.file, e.line),
        text: e.message.trim().slice(0, 90),
        resolution: 'Fix the type error — do not suppress.',
      })));
    }
    return { violations, errors };
  },
};
