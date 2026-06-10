import {
  mkdirSync, writeFileSync, readFileSync, copyFileSync, existsSync, readdirSync,
} from 'node:fs';
import { join, relative } from 'node:path';

const ENGINE_ROOT = '/home/user/Projects/slop-gate';
const COMMIT_HOOK = `${ENGINE_ROOT}/hooks/commit-hook.sh`;
const EDIT_HOOK = `${ENGINE_ROOT}/hooks/edit-hook.sh`;

const EXCLUDE_SCAN = new Set([
  'node_modules', '.next', '.open-next', '.astro', 'dist', '.worktrees',
]);
const SCAN_BASES = ['', 'apps', 'packages', 'workers'];
const EXT_CANDIDATES = ['.ts', '.tsx', '.astro', '.js', '.jsx', '.vue', '.svelte'];
const OPTIONAL_SKIP = ['.next', '.open-next', '.astro', 'build', '.turbo', 'coverage'];
const BASE_SKIP = ['node_modules', 'dist', 'tests', '.worktrees'];

const PRE_TOOL = { matcher: 'Bash', command: COMMIT_HOOK };
const POST_TOOL = { matcher: 'Edit|Write', command: EDIT_HOOK };

/** @param {string} targetDir */
function readPackageJson(targetDir) {
  const p = join(targetDir, 'package.json');
  if (!existsSync(p)) return null;
  try { return JSON.parse(readFileSync(p, 'utf8')); } catch { return null; }
}

/** @param {string} targetDir @param {string[]} patterns */
function expandWorkspaceGlobs(targetDir, patterns) {
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
function workspacePatterns(pkg) {
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

/** @param {{ roots: string[], exts: string[], skipDirs: string[] }} detected */
function formatConfig(detected) {
  return `// generated by slop-gate init — review roots + add project rule packs (see convention-sources.json).
// slop-gate project config. Engine is global+auto-latest; THIS file is pinned per project.
export default {
  roots: ${JSON.stringify(detected.roots)},
  exts: ${JSON.stringify(detected.exts)},
  skipDirs: ${JSON.stringify(detected.skipDirs)},

  // baseline packs this project OPTS INTO (nothing fires until listed)
  baseline: ['no-stubs', 'ts-suppress', 'as-any'],

  // project-owned rule packs (pinned, in this repo)
  rules: [],                      // e.g. ['./rules/my-rule.mjs']
  astRules: './rules/ast',        // dir of *.yml (optional)

  gate: { file: ['critical', 'high'], staged: ['critical', 'high'] },
  suppressions: './suppressions.json',
  fixtures: './fixtures',
};
`;
}

/**
 * @param {unknown} settings
 * @param {{ matcher: string, command: string }} spec
 */
function ensureHookEntry(settings, event, spec) {
  if (!settings.hooks) settings.hooks = {};
  if (!Array.isArray(settings.hooks[event])) settings.hooks[event] = [];

  let entry = settings.hooks[event].find((e) => e?.matcher === spec.matcher);
  if (!entry) {
    entry = { matcher: spec.matcher, hooks: [] };
    settings.hooks[event].push(entry);
  }
  if (!Array.isArray(entry.hooks)) entry.hooks = [];

  const present = entry.hooks.some((h) => h?.type === 'command' && h?.command === spec.command);
  if (!present) entry.hooks.push({ type: 'command', command: spec.command });
  return !present;
}

/**
 * @param {string} targetDir
 * @returns {'created' | 'merged' | 'already-present'}
 */
export function mergeSettingsJson(targetDir) {
  const claudeDir = join(targetDir, '.claude');
  const settingsPath = join(claudeDir, 'settings.json');
  mkdirSync(claudeDir, { recursive: true });

  if (!existsSync(settingsPath)) {
    const settings = {
      hooks: {
        PreToolUse: [{ matcher: PRE_TOOL.matcher, hooks: [{ type: 'command', command: PRE_TOOL.command }] }],
        PostToolUse: [{ matcher: POST_TOOL.matcher, hooks: [{ type: 'command', command: POST_TOOL.command }] }],
      },
    };
    writeFileSync(settingsPath, `${JSON.stringify(settings, null, 2)}\n`);
    return 'created';
  }

  const raw = readFileSync(settingsPath, 'utf8');
  const settings = JSON.parse(raw);
  const addedPre = ensureHookEntry(settings, 'PreToolUse', PRE_TOOL);
  const addedPost = ensureHookEntry(settings, 'PostToolUse', POST_TOOL);

  if (!addedPre && !addedPost) return 'already-present';

  copyFileSync(settingsPath, `${settingsPath}.bak`);
  writeFileSync(settingsPath, `${JSON.stringify(settings, null, 2)}\n`);
  return 'merged';
}

/** @param {string} targetDir @param {{ quiet?: boolean }} [options] */
export function runInit(targetDir, options = {}) {
  const base = join(targetDir, '.slop-gate');
  const configPath = join(base, 'config.mjs');
  const configExists = existsSync(configPath);

  const { roots, warned } = detectRoots(targetDir);
  const exts = detectExts(targetDir, roots);
  const skipDirs = detectSkipDirs(targetDir);

  mkdirSync(join(base, 'rules/ast'), { recursive: true });

  if (!configExists) {
    mkdirSync(join(base, 'fixtures/src'), { recursive: true });
    writeFileSync(configPath, formatConfig({ roots, exts, skipDirs }));
    writeFileSync(
      join(base, 'suppressions.json'),
      `${JSON.stringify({ version: 1, entries: [] }, null, 2)}\n`,
    );
  }

  writeFileSync(
    join(base, 'convention-sources.json'),
    `${JSON.stringify(buildConventionSources(targetDir), null, 2)}\n`,
  );

  const settingsAction = mergeSettingsJson(targetDir);

  if (!options.quiet) {
    if (configExists) {
      process.stderr.write(`slop-gate: ${configPath} already exists — preserved (not overwritten)\n`);
    } else {
      process.stdout.write(`slop-gate: scaffolded ${base}/\n`);
    }
    if (warned) {
      process.stderr.write('slop-gate: WARNING — no source roots detected; defaulting to ["src"] — review manually\n');
    }
    process.stdout.write('\n--- slop-gate init summary ---\n');
    process.stdout.write(`roots:     ${JSON.stringify(roots)}\n`);
    process.stdout.write(`exts:      ${JSON.stringify(exts)}\n`);
    process.stdout.write(`skipDirs:  ${JSON.stringify(skipDirs)}\n`);
    process.stdout.write(`settings:  ${settingsAction} (.claude/settings.json)\n`);
    process.stdout.write('\nNEXT STEPS:\n');
    process.stdout.write('  1. Review .slop-gate/convention-sources.json for project rule candidates\n');
    process.stdout.write('  2. Author project rule packs in .slop-gate/rules/\n');
    process.stdout.write('  3. Run a dry-run gate pass before enabling blocking mode\n');
    process.stdout.write('  4. Drive each new rule to zero hits before adding to baseline/rules\n');
  }

  return 0;
}