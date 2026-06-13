import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const BIN = new URL('../bin/slopgate', import.meta.url).pathname;

function repo() {
  const root = mkdtempSync(join(tmpdir(), 'slopgate-cli-'));
  spawnSync('git', ['init'], { cwd: root, stdio: 'ignore' });
  mkdirSync(join(root, 'src'));
  return root;
}

function writeConfig(root, name = 'slopgate.config.mjs') {
  const configPath = join(root, name);
  writeFileSync(configPath, `export default { repoRoot: ${JSON.stringify(root)}, roots: ['src'], exts: ['.ts'], baseline: ['as-any'], rules: [], gate: { file: ['critical', 'high'], staged: ['critical', 'high'] } };\n`);
  return configPath;
}

function run(args, cwd = process.cwd()) {
  return spawnSync(process.execPath, [BIN, ...args], { cwd, encoding: 'utf8' });
}

test('--version exits without config', () => {
  const r = run(['--version']);

  assert.equal(r.status, 0);
  assert.match(r.stdout, /^slopgate 1\.0\.0\n$/);
  assert.equal(r.stderr, '');
});

test('--config value stats is not treated as the stats command', () => {
  const root = repo();
  const configPath = writeConfig(root, 'stats');
  writeFileSync(join(root, 'src/bad.ts'), 'const x = foo as any;\n');

  const r = run(['--config', configPath, '--file', 'src/bad.ts'], root);

  assert.equal(r.status, 1);
  assert.match(r.stderr, /src\/bad\.ts:1/);
  assert.doesNotMatch(r.stdout, /incident\(s\) stopped/);
});

test('--config value init is not treated as the init command', () => {
  const root = repo();
  const configPath = writeConfig(root, 'init');
  writeFileSync(join(root, 'src/bad.ts'), 'const x = foo as any;\n');

  const r = run(['--config', configPath, '--file', 'src/bad.ts'], root);

  assert.equal(r.status, 1);
  assert.match(r.stderr, /src\/bad\.ts:1/);
  assert.doesNotMatch(r.stdout, /scaffolded/);
});
