import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ENGINE_ROOT } from '../install-hooks.mjs';
import { mergeSettingsJson } from './scaffold.mjs';

const SESSION_HOOK = `${ENGINE_ROOT}/hooks/session-start.sh`;

function tempDir() {
  return mkdtempSync(join(tmpdir(), 'sg-scaffold-'));
}

test('fresh create: SessionStart matcher-less + PreToolUse + PostToolUse', () => {
  const dir = tempDir();
  assert.equal(mergeSettingsJson(dir), 'created');

  const settings = JSON.parse(readFileSync(join(dir, '.claude', 'settings.json'), 'utf8'));
  assert.ok(Array.isArray(settings.hooks.PreToolUse));
  assert.ok(Array.isArray(settings.hooks.PostToolUse));
  assert.ok(Array.isArray(settings.hooks.SessionStart));
  assert.equal(settings.hooks.SessionStart.length, 1);

  const entry = settings.hooks.SessionStart[0];
  assert.equal(entry.hooks[0].command, SESSION_HOOK);
  assert.equal('matcher' in entry, false);
});

test('merge into existing settings: SessionStart added, content preserved, .bak written', () => {
  const dir = tempDir();
  const claudeDir = join(dir, '.claude');
  mkdirSync(claudeDir, { recursive: true });
  const settingsPath = join(claudeDir, 'settings.json');
  const existing = {
    hooks: {
      PreToolUse: [{
        matcher: 'Custom',
        hooks: [{ type: 'command', command: '/usr/local/bin/custom-pre' }],
      }],
    },
    permissions: { allow: ['Bash'] },
  };
  writeFileSync(settingsPath, `${JSON.stringify(existing, null, 2)}\n`);

  assert.equal(mergeSettingsJson(dir), 'merged');
  assert.ok(existsSync(`${settingsPath}.bak`));

  const settings = JSON.parse(readFileSync(settingsPath, 'utf8'));
  assert.deepEqual(settings.permissions, { allow: ['Bash'] });
  assert.equal(
    settings.hooks.PreToolUse.find((e) => e.matcher === 'Custom').hooks[0].command,
    '/usr/local/bin/custom-pre',
  );
  assert.equal(settings.hooks.SessionStart.length, 1);
  assert.equal(settings.hooks.SessionStart[0].hooks[0].command, SESSION_HOOK);
  assert.equal('matcher' in settings.hooks.SessionStart[0], false);
});

test('idempotent merge: SessionStart not duplicated on second run', () => {
  const dir = tempDir();
  const settingsPath = join(dir, '.claude', 'settings.json');

  assert.equal(mergeSettingsJson(dir), 'created');
  assert.equal(mergeSettingsJson(dir), 'already-present');

  const settings = JSON.parse(readFileSync(settingsPath, 'utf8'));
  const commands = settings.hooks.SessionStart.flatMap((e) => e.hooks.map((h) => h.command));
  assert.deepEqual(commands, [SESSION_HOOK]);
});
