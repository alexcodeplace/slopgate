import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const CONFIG_TEMPLATE = `// slop-gate project config. Engine is global+auto-latest; THIS file is pinned per project.
export default {
  roots: ['src'],                 // repo-relative source dirs to scan
  exts: ['.ts', '.tsx', '.astro'],
  skipDirs: ['node_modules', 'dist', 'tests', '.worktrees'],

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

const HOOK_SNIPPET = `Add to this project's .claude/settings.json:
{
  "hooks": {
    "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "/home/user/Projects/slop-gate/hooks/commit-hook.sh" }] }],
    "PostToolUse": [{ "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": "/home/user/Projects/slop-gate/hooks/edit-hook.sh" }] }]
  }
}`;

export function runInit(targetDir) {
  const base = join(targetDir, '.slop-gate');
  if (existsSync(join(base, 'config.mjs'))) {
    process.stderr.write(`slop-gate: ${base}/config.mjs already exists — not overwriting\n`);
    return 1;
  }
  mkdirSync(join(base, 'rules/ast'), { recursive: true });
  mkdirSync(join(base, 'fixtures/src'), { recursive: true });
  writeFileSync(join(base, 'config.mjs'), CONFIG_TEMPLATE);
  writeFileSync(join(base, 'suppressions.json'), JSON.stringify({ version: 1, entries: [] }, null, 2) + '\n');
  process.stdout.write(`slop-gate: scaffolded ${base}/\n\n${HOOK_SNIPPET}\n`);
  return 0;
}