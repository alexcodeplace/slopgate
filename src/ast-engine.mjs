/**
 * ast-grep engine wrapper (bucket-B structural rules).
 * Returns findings in the shared violation shape.
 * Graceful degradation: missing binary → { available:false } — caller warns, never bricks.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

export function resolveAstGrepBin(repoRoot) {
  const local = join(repoRoot, 'node_modules/.bin/ast-grep');
  if (existsSync(local)) return { bin: local, source: 'local' };
  const probe = spawnSync('ast-grep', ['--version'], { encoding: 'utf8' });
  if (probe.status === 0) return { bin: 'ast-grep', source: 'path' };
  return null;
}

/**
 * @param {import('./config.mjs').ResolvedConfig} config
 * @param {string[]|null} files - repo-relative targets (ts/tsx only), or null = scan config roots
 * @returns {{ available:boolean, violations:any[], errors:string[] }}
 */
export function runAstGrepScan(config, files = null, opts = {}) {
  const ruleDirs = (config.astRuleDirs || []).filter(existsSync);
  if (ruleDirs.length === 0) return { available: true, violations: [], errors: [] };

  const resolved = resolveAstGrepBin(config.repoRoot);
  if (!resolved) {
    return { available: false, violations: [], errors: ['ast-grep binary not found (npm i -g @ast-grep/cli) — bucket-B rules SKIPPED'] };
  }
  const bin = resolved.bin;
  const errors = [];
  if (resolved.source === 'path') errors.push('ast-grep: using PATH binary (version not pinned — results may differ from CI)');

  // ast-grep reads ruleDirs from an sgconfig.yml; synthesize one pointing at all rule dirs.
  const dir = mkdtempSync(join(tmpdir(), 'slopgate-sg-'));
  try {
    const sgConfig = join(dir, 'sgconfig.yml');
    writeFileSync(sgConfig, 'ruleDirs:\n' + ruleDirs.map((d) => `  - ${d}`).join('\n') + '\n');

    const targets = files === null ? config.rootsRel : (opts.rawTargets ? files : files.filter((f) => /\.(ts|tsx)$/.test(f)));
    if (files !== null && targets.length === 0) return { available: true, violations: [], errors: [] };

    const res = spawnSync(bin, ['scan', '--config', sgConfig, '--json', ...targets], {
      encoding: 'utf8', cwd: config.repoRoot, maxBuffer: 32 * 1024 * 1024, timeout: 60_000,
    });
    if (res.error || res.stdout == null) {
      return { available: false, violations: [], errors: [`ast-grep failed: ${res.error || res.stderr?.slice(0, 300)}`] };
    }
    let matches;
    try { matches = JSON.parse(res.stdout); } catch (e) {
      return { available: true, violations: [], errors: [`ast-grep JSON parse error: ${e}`] };
    }
    if (!Array.isArray(matches)) {
      return { available: true, violations: [], errors: ['ast-grep output was not an array'] };
    }
    const violations = [];
    if (res.stderr && /error/i.test(res.stderr) && !/error\(s\) found in code/i.test(res.stderr)) {
      errors.push(`ast-grep stderr: ${res.stderr.slice(0, 500)}`);
    }
    for (const m of matches) {
      let meta = {};
      try { meta = JSON.parse(m.note || '{}'); } catch { errors.push(`rule ${m.ruleId}: note is not valid JSON`); }
      const firstLine = (m.lines || '').split('\n')[0];
      violations.push({
        id: m.ruleId,
        severity: meta.severity || (m.severity === 'error' ? 'high' : 'medium'),
        category: meta.category || 'convention',
        file: m.file,
        line: (m.range?.start?.line ?? 0) + 1,
        fullLine: firstLine,
        text: firstLine.trim().slice(0, 90),
        resolution: meta.resolution || m.message || '',
        engine: 'ast',
      });
    }
    return { available: true, violations, errors };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}