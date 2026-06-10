import { readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';

const EXCLUDE_SCAN = new Set([
  'node_modules', '.next', '.open-next', '.astro', 'dist', '.worktrees',
]);
const SCAN_BASES = ['', 'apps', 'packages', 'workers'];
const EXT_CANDIDATES = ['.ts', '.tsx', '.astro', '.js', '.jsx', '.vue', '.svelte'];
const OPTIONAL_SKIP = ['.next', '.open-next', '.astro', 'build', '.turbo', 'coverage'];
const BASE_SKIP = ['node_modules', 'dist', 'tests', '.worktrees'];

/** @param {string} targetDir */
export function readPackageJson(targetDir) {
  const p = join(targetDir, 'package.json');
  if (!existsSync(p)) return null;
  try { return JSON.parse(readFileSync(p, 'utf8')); } catch { return null; }
}

/** @param {string} targetDir @param {string[]} patterns */
export function expandWorkspaceGlobs(targetDir, patterns) {
  const out = [];
  for (const pattern of patterns) {
    const norm = pattern.replace(/\\/g, '/');
    const star = norm.indexOf('*');
    if (star === -1) { out.push(norm); continue; }
    const base = norm.slice(0, star);
    const basePath = join(targetDir, base);
    if (!existsSync(basePath)) continue;
    for (const ent of readdirSync(basePath, { withFileTypes: true })) {
      if (ent.isDirectory() && !EXCLUDE_SCAN.has(ent.name)) {
        out.push(`${base}${ent.name}`.replace(/\/+/g, '/'));
      }
    }
  }
  return out;
}

/** @param {string} targetDir */
export function workspacePatterns(pkg) {
  if (!pkg?.workspaces) return [];
  if (Array.isArray(pkg.workspaces)) return pkg.workspaces;
  if (Array.isArray(pkg.workspaces.packages)) return pkg.workspaces.packages;
  return [];
}

/**
 * @param {string} dir abs path
 * @param {string} targetDir
 * @param {number} depth remaining
 * @param {Set<string>} found repo-relative src dir paths
 */
function findSrcDirs(dir, targetDir, depth, found) {
  if (depth < 0 || !existsSync(dir)) return;
  const rel = relative(targetDir, dir).replace(/\\/g, '/') || '.';
  if (rel !== '.' && rel.endsWith('/src')) found.add(rel);
  if (depth === 0) return;
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    if (!ent.isDirectory() || EXCLUDE_SCAN.has(ent.name)) continue;
    findSrcDirs(join(dir, ent.name), targetDir, depth - 1, found);
  }
}

/** @param {string} targetDir @returns {{ roots: string[], warned: boolean }} */
export function detectRoots(targetDir) {
  const found = new Set();

  const pkg = readPackageJson(targetDir);
  for (const ws of expandWorkspaceGlobs(targetDir, workspacePatterns(pkg))) {
    const srcRel = `${ws}/src`.replace(/\/+/g, '/');
    if (existsSync(join(targetDir, srcRel))) found.add(srcRel);
  }

  if (existsSync(join(targetDir, 'src'))) found.add('src');

  for (const base of SCAN_BASES) {
    const start = base ? join(targetDir, base) : targetDir;
    const maxDepth = base ? 2 : 3;
    findSrcDirs(start, targetDir, maxDepth, found);
  }

  const roots = [...found].sort();
  if (roots.length) return { roots, warned: false };
  return { roots: ['src'], warned: true };
}

/**
 * @param {string} targetDir
 * @param {string[]} roots
 * @returns {string[]}
 */
export function detectExts(targetDir, roots) {
  const counts = Object.fromEntries(EXT_CANDIDATES.map((e) => [e, 0]));

  const walk = (dir) => {
    if (!existsSync(dir)) return;
    let entries;
    try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const ent of entries) {
      if (ent.isDirectory()) {
        if (EXCLUDE_SCAN.has(ent.name) || BASE_SKIP.includes(ent.name)) continue;
        walk(join(dir, ent.name));
      } else {
        const ext = ent.name.includes('.') ? `.${ent.name.split('.').pop()}` : '';
        if (ext in counts) counts[ext]++;
      }
    }
  };

  for (const root of roots) walk(join(targetDir, root));

  const detected = EXT_CANDIDATES.filter((e) => counts[e] > 0);
  if (detected.length === 0) return ['.ts', '.tsx', '.astro'];
  return detected;
}

/** @param {string} targetDir @returns {string[]} */
export function detectSkipDirs(targetDir) {
  const skip = [...BASE_SKIP];
  for (const d of OPTIONAL_SKIP) {
    if (existsSync(join(targetDir, d)) && !skip.includes(d)) skip.push(d);
  }
  return skip;
}

/** @param {string} targetDir */
export function detectCheckers(targetDir) {
  const bin = (name) => existsSync(join(targetDir, 'node_modules/.bin', name));
  const checkers = {};
  if (existsSync(join(targetDir, 'tsconfig.json')) && bin('tsc')) checkers.tsc = true;
  if (bin('knip')) checkers.knip = true;
  if (bin('jscpd')) checkers.jscpd = { minTokens: 50 };
  if (bin('depcruise')) checkers.depcruise = true;
  if (bin('type-coverage')) checkers['type-coverage'] = true;
  checkers['diff-shape'] = { maxDirs: 5 };
  return checkers;
}

/**
 * @param {string} dir abs
 * @param {string} targetDir
 * @param {number} depth
 * @param {string} name
 * @param {string[]} out repo-relative paths
 */
function collectNamedFiles(dir, targetDir, depth, name, out) {
  if (depth < 0 || !existsSync(dir)) return;
  const candidate = join(dir, name);
  if (existsSync(candidate)) {
    out.push(relative(targetDir, candidate).replace(/\\/g, '/'));
  }
  if (depth === 0) return;
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    if (!ent.isDirectory() || ent.name === 'node_modules' || EXCLUDE_SCAN.has(ent.name)) continue;
    collectNamedFiles(join(dir, ent.name), targetDir, depth - 1, name, out);
  }
}

/** @param {string} dir abs @param {string} targetDir @param {string[]} out */
function collectDirFiles(dir, targetDir, out) {
  if (!existsSync(dir)) return;
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const ent of entries) {
    const abs = join(dir, ent.name);
    if (ent.isDirectory()) collectDirFiles(abs, targetDir, out);
    else out.push(relative(targetDir, abs).replace(/\\/g, '/'));
  }
}

/** @param {string} targetDir */
export function buildConventionSources(targetDir) {
  const claudeMd = [];
  collectNamedFiles(targetDir, targetDir, 3, 'CLAUDE.md', claudeMd);

  const skills = [];
  collectDirFiles(join(targetDir, '.claude/skills'), targetDir, skills);
  const agents = [];
  collectDirFiles(join(targetDir, '.claude/agents'), targetDir, agents);
  const commands = [];
  collectDirFiles(join(targetDir, '.claude/commands'), targetDir, commands);

  const editorRules = [];
  for (const f of ['.cursorrules', '.windsurfrules', '.clinerules']) {
    if (existsSync(join(targetDir, f))) editorRules.push(f);
  }

  const knowledgeDocs = [];
  for (const f of ['.project_knowledge.md', 'AGENTS.md']) {
    if (existsSync(join(targetDir, f))) knowledgeDocs.push(f);
  }

  const sort = (a) => [...a].sort();
  return {
    version: 1,
    generated_hint: 'inputs an agent should read to derive project-specific rule candidates',
    claudeMd: sort(claudeMd),
    skills: sort(skills),
    agents: sort(agents),
    commands: sort(commands),
    editorRules: sort(editorRules),
    knowledgeDocs: sort(knowledgeDocs),
  };
}
