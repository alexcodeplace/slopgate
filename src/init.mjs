import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { installPreCommitHook } from './install-hooks.mjs';
import { installSkills } from './install-skills.mjs';
import {
  detectRoots,
  detectExts,
  detectSkipDirs,
  detectCheckers,
  buildConventionSources,
} from './init/detect-stack.mjs';
import { DEPCRUISE_STARTER, formatConfig, mergeSettingsJson } from './init/scaffold.mjs';

/** @param {string} targetDir @param {{ quiet?: boolean }} [options] */
export function runInit(targetDir, options = {}) {
  const base = join(targetDir, '.slopgate');
  const configPath = join(base, 'config.mjs');
  const configExists = existsSync(configPath);

  const { roots, warned } = detectRoots(targetDir);
  const exts = detectExts(targetDir, roots);
  const skipDirs = detectSkipDirs(targetDir);

  mkdirSync(join(base, 'rules/ast'), { recursive: true });

  const checkers = detectCheckers(targetDir);

  if (!configExists) {
    mkdirSync(join(base, 'fixtures/src'), { recursive: true });
    writeFileSync(configPath, formatConfig({ roots, exts, skipDirs, checkers }));
    writeFileSync(
      join(base, 'suppressions.json'),
      `${JSON.stringify({ version: 1, entries: [] }, null, 2)}\n`,
    );
  }
  const depcruisePath = join(base, 'depcruise.cjs');
  if (checkers.depcruise && !existsSync(depcruisePath)) writeFileSync(depcruisePath, DEPCRUISE_STARTER);

  let hookAction = 'skipped (not a git repo)';
  try { hookAction = installPreCommitHook(targetDir).action; } catch { /* not a git repo */ }

  installSkills();

  writeFileSync(
    join(base, 'convention-sources.json'),
    `${JSON.stringify(buildConventionSources(targetDir), null, 2)}\n`,
  );

  const settingsAction = mergeSettingsJson(targetDir);

  if (!options.quiet) {
    if (configExists) {
      process.stderr.write(`slopgate: ${configPath} already exists — preserved (not overwritten)\n`);
    } else {
      process.stdout.write(`slopgate: scaffolded ${base}/\n`);
    }
    if (warned) {
      process.stderr.write('slopgate: WARNING — no source roots detected; defaulting to ["src"] — review manually\n');
    }
    process.stdout.write('\n--- slopgate init summary ---\n');
    process.stdout.write(`roots:     ${JSON.stringify(roots)}\n`);
    process.stdout.write(`exts:      ${JSON.stringify(exts)}\n`);
    process.stdout.write(`skipDirs:  ${JSON.stringify(skipDirs)}\n`);
    process.stdout.write(`settings:  ${settingsAction} (.claude/settings.json)\n`);
    process.stdout.write(`checkers:  ${JSON.stringify(Object.keys(checkers))}\n`);
    process.stdout.write(`pre-commit hook: ${hookAction}\n`);
    process.stdout.write('\nNEXT STEPS:\n');
    process.stdout.write('  1. Review .slopgate/convention-sources.json for project rule candidates\n');
    process.stdout.write('  2. Author project rule packs in .slopgate/rules/\n');
    process.stdout.write('  3. Run a dry-run gate pass before enabling blocking mode\n');
    process.stdout.write('  4. Drive each new rule to zero hits before adding to baseline/rules\n');
    process.stdout.write('  5. Run: slopgate baseline --config .slopgate/config.mjs (absorb pre-existing violations)\n');
  }

  return 0;
}
