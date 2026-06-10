import { readdirSync, existsSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { join, relative, extname } from 'node:path';

function isTestFile(p) { return /\.test\.(ts|tsx)$/.test(p); }

/**
 * @param {import('./config.mjs').ResolvedConfig} config
 * @param {{ staged?:boolean, file?:string }} [opts]
 * @returns {string[]} repo-relative source paths
 */
export function listSourceFiles(config, opts = {}) {
  if (opts.file) {
    const rel = opts.file.startsWith('/') ? relative(config.repoRoot, opts.file) : opts.file;
    const underRoot = config.rootsRel.some((r) => rel === r || rel.startsWith(r + '/'));
    const ok = underRoot && config.exts.has(extname(rel)) && !isTestFile(rel) && existsSync(join(config.repoRoot, rel));
    return ok ? [rel] : [];
  }

  if (opts.staged) {
    try {
      const raw = execSync('git diff --cached --name-only', { encoding: 'utf8', cwd: config.repoRoot });
      return raw.trim().split('\n').filter(Boolean).filter((f) => {
        const underRoot = config.rootsRel.some((r) => f === r || f.startsWith(r + '/'));
        return underRoot && config.exts.has(extname(f)) && !isTestFile(f);
      });
    } catch { return []; }
  }

  const files = [];
  const walk = (dir) => {
    if (!existsSync(dir)) return;
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      if (config.skipDirs.has(ent.name)) continue;
      const p = join(dir, ent.name);
      if (ent.isDirectory()) walk(p);
      else if (config.exts.has(extname(ent.name)) && !isTestFile(ent.name)) files.push(relative(config.repoRoot, p));
    }
  };
  for (const root of config.roots) walk(root);
  return files.sort();
}