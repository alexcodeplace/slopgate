// src/checkers/shared.mjs
/** Shared checker plumbing: local-bin resolution (no global/npx — deterministic versions),
 *  spawn wrapper (timeout → ok:false, never throws), source-line lookup for lineHash. */
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';

export function localBin(repoRoot, name) {
  const p = join(repoRoot, 'node_modules/.bin', name);
  return existsSync(p) ? p : null;
}

export function runTool(bin, args, { cwd, timeout }) {
  const res = spawnSync(bin, args, { encoding: 'utf8', cwd, timeout, maxBuffer: 64 * 1024 * 1024 });
  if (res.error || res.signal) return { ok: false, error: res.error ? String(res.error) : `killed by signal ${res.signal}`, stdout: res.stdout ?? '', stderr: res.stderr ?? '', status: null };
  return { ok: true, error: null, stdout: res.stdout ?? '', stderr: res.stderr ?? '', status: res.status };
}

export function sourceLine(repoRoot, file, line) {
  try { return readFileSync(join(repoRoot, file), 'utf8').split('\n')[line - 1] ?? ''; }
  catch { return ''; }
}
