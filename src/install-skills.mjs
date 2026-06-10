// src/install-skills.mjs
import { existsSync, mkdirSync, readdirSync, cpSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';

const ENGINE_ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const SKILLS_SRC = join(ENGINE_ROOT, 'skills');

export function installSkills({ force = false, dest } = {}) {
  const skillsDest = dest ?? join(homedir(), '.claude', 'skills');
  const results = [];

  if (!existsSync(SKILLS_SRC)) return results;

  const skillDirs = readdirSync(SKILLS_SRC, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name);

  for (const name of skillDirs) {
    const target = join(skillsDest, name);
    if (existsSync(target) && !force) {
      results.push({ name, action: 'skipped' });
      continue;
    }
    mkdirSync(target, { recursive: true });
    cpSync(join(SKILLS_SRC, name), target, { recursive: true, force: true });
    results.push({ name, action: force && existsSync(target) ? 'updated' : 'installed' });
  }

  return results;
}
