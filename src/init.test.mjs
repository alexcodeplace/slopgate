import { mkdirSync, writeFileSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { execSync } from 'node:child_process';
import { runInit } from './init.mjs';
import { ENGINE_ROOT } from './install-hooks.mjs';

const FIXTURE = '/home/user/Projects/slop-gate/.tmp-inittest';
const COMMIT_HOOK = `${ENGINE_ROOT}/hooks/commit-hook.sh`;
const EDIT_HOOK = `${ENGINE_ROOT}/hooks/edit-hook.sh`;

function assert(label, ok) {
  console.log(`${ok ? 'PASS' : 'FAIL'}: ${label}`);
  return ok;
}

function setupFixture() {
  rmSync(FIXTURE, { recursive: true, force: true });
  mkdirSync(join(FIXTURE, 'apps/web/src'), { recursive: true });
  mkdirSync(join(FIXTURE, 'packages/ui/src'), { recursive: true });
  mkdirSync(join(FIXTURE, '.next'), { recursive: true });
  mkdirSync(join(FIXTURE, '.claude/skills'), { recursive: true });

  writeFileSync(join(FIXTURE, 'apps/web/src/x.tsx'), 'export const x = 1;\n');
  writeFileSync(join(FIXTURE, 'packages/ui/src/y.ts'), 'export const y = 1;\n');
  writeFileSync(
    join(FIXTURE, 'package.json'),
    `${JSON.stringify({ workspaces: ['apps/*', 'packages/*'] }, null, 2)}\n`,
  );
  writeFileSync(join(FIXTURE, 'CLAUDE.md'), '# test\n');
  writeFileSync(join(FIXTURE, '.claude/skills/foo.md'), '# skill\n');
  writeFileSync(join(FIXTURE, '.cursorrules'), 'rules\n');
  execSync('git init -q', { cwd: FIXTURE });
  writeFileSync(
    join(FIXTURE, '.claude/settings.json'),
    `${JSON.stringify({
      hooks: {
        PostToolUse: [{
          matcher: 'Edit|Write',
          hooks: [{ type: 'command', command: 'code-review-graph.sh' }],
        }],
      },
    }, null, 2)}\n`,
  );
}

async function loadConfig(dir) {
  const mod = await import(pathToFileURL(join(dir, '.slop-gate/config.mjs')).href);
  return mod.default;
}

function countHookCommands(settings, event, command) {
  let n = 0;
  for (const entry of settings.hooks?.[event] ?? []) {
    for (const h of entry.hooks ?? []) {
      if (h.type === 'command' && h.command === command) n++;
    }
  }
  return n;
}

function hasHookCommand(settings, event, command) {
  return countHookCommands(settings, event, command) > 0;
}

async function main() {
  let allPass = true;
  const fail = (label) => { allPass = assert(label, false) && allPass; };
  const pass = (label) => { assert(label, true); };

  setupFixture();
  runInit(FIXTURE, { quiet: true });

  const config = await loadConfig(FIXTURE);
  if (!assert('config roots == ["apps/web/src","packages/ui/src"]',
    JSON.stringify(config.roots) === JSON.stringify(['apps/web/src', 'packages/ui/src']))) allPass = false;
  if (!assert('skipDirs includes ".next"', config.skipDirs.includes('.next'))) allPass = false;
  if (!assert('exts includes ".tsx"', config.exts.includes('.tsx'))) allPass = false;
  if (!assert('exts includes ".ts"', config.exts.includes('.ts'))) allPass = false;

  const sources = JSON.parse(readFileSync(join(FIXTURE, '.slop-gate/convention-sources.json'), 'utf8'));
  if (!assert('convention-sources lists CLAUDE.md', sources.claudeMd.includes('CLAUDE.md'))) allPass = false;
  if (!assert('convention-sources lists skills/foo.md', sources.skills.includes('.claude/skills/foo.md'))) allPass = false;
  if (!assert('convention-sources lists .cursorrules', sources.editorRules.includes('.cursorrules'))) allPass = false;

  const settings = JSON.parse(readFileSync(join(FIXTURE, '.claude/settings.json'), 'utf8'));
  if (!assert('settings still has code-review-graph hook',
    hasHookCommand(settings, 'PostToolUse', 'code-review-graph.sh'))) allPass = false;
  if (!assert('settings has edit-hook entry',
    hasHookCommand(settings, 'PostToolUse', EDIT_HOOK))) allPass = false;
  if (!assert('settings has commit-hook PreToolUse entry',
    hasHookCommand(settings, 'PreToolUse', COMMIT_HOOK))) allPass = false;
  if (!assert('.bak exists', existsSync(join(FIXTURE, '.claude/settings.json.bak')))) allPass = false;

  const cfgText = readFileSync(join(FIXTURE, '.slop-gate/config.mjs'), 'utf8');
  if (!assert('config has checkers block', cfgText.includes('checkers: {'))) allPass = false;
  if (!assert("config has diff-shape default", cfgText.includes("'diff-shape': {\"maxDirs\":5}"))) allPass = false;
  if (!assert('config has astDisable', cfgText.includes('astDisable: []'))) allPass = false;
  if (!assert('pre-commit hook installed', existsSync(join(FIXTURE, '.git/hooks/pre-commit')))) allPass = false;

  const configBefore = cfgText;
  runInit(FIXTURE, { quiet: true });
  const configAfter = readFileSync(join(FIXTURE, '.slop-gate/config.mjs'), 'utf8');
  if (!assert('idempotent: config unchanged', configBefore === configAfter)) allPass = false;

  const settings2 = JSON.parse(readFileSync(join(FIXTURE, '.claude/settings.json'), 'utf8'));
  if (!assert('idempotent: no duplicate edit-hook',
    countHookCommands(settings2, 'PostToolUse', EDIT_HOOK) === 1)) allPass = false;
  if (!assert('idempotent: no duplicate commit-hook',
    countHookCommands(settings2, 'PreToolUse', COMMIT_HOOK) === 1)) allPass = false;

  console.log(`\nDETECTION_SAMPLE: roots=${JSON.stringify(config.roots)} exts=${JSON.stringify(config.exts)} skipDirs=${JSON.stringify(config.skipDirs)}`);
  console.log(`OVERALL: ${allPass ? 'PASS' : 'FAIL'}`);

  rmSync(FIXTURE, { recursive: true, force: true });
  process.exit(allPass ? 0 : 1);
}

main().catch((e) => { console.error(e); process.exit(1); });