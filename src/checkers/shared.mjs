// src/checkers/shared.mjs
/** Shared checker plumbing: local-bin resolution (no global/npx — deterministic versions),
 *  spawn wrapper (timeout → ok:false, never throws), source-line lookup for lineHash. */
import { existsSync, readFileSync } from 'node:fs';
import { spawnSync, execFile } from 'node:child_process';
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

/** Async sibling of runTool. Same return shape; never rejects. */
export function runToolAsync(bin, args, { cwd, timeout } = {}) {
  return new Promise((resolve) => {
    execFile(bin, args, { encoding: 'utf8', cwd, timeout, maxBuffer: 64 * 1024 * 1024 }, (err, stdout, stderr) => {
      if (err) {
        const killed = err.killed || err.signal;
        resolve({ ok: false, error: killed ? `killed by signal ${err.signal}` : String(err), stdout: stdout ?? '', stderr: stderr ?? '', status: typeof err.code === 'number' ? err.code : null });
      } else {
        resolve({ ok: true, error: null, stdout: stdout ?? '', stderr: stderr ?? '', status: 0 });
      }
    });
  });
}

/** Run a tool that emits JSON on stdout. Never rejects. */
export async function runJsonTool(label, bin, args, opts) {
  const res = await runToolAsync(bin, args, opts);
  if (!res.ok) return { data: null, errors: [`${label} failed: ${res.error}`] };
  try { return { data: JSON.parse(res.stdout), errors: [] }; }
  catch (e) { return { data: null, errors: [`${label} JSON parse error: ${e}`] }; }
}

/** Bounded-concurrency map; preserves input order in the result array. */
export async function mapLimit(items, limit, fn) {
  const results = new Array(items.length);
  let next = 0;
  async function worker() {
    while (next < items.length) {
      const i = next++;
      results[i] = await fn(items[i], i);
    }
  }
  const n = Math.max(1, Math.min(limit, items.length));
  await Promise.all(Array.from({ length: n }, worker));
  return results;
}
