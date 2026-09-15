#!/usr/bin/env node
// Explicit CI/developer setup, never called by a scan or hook.
import { mkdirSync, existsSync, appendFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';

const root = resolve(process.env.SLOPGATE_TEST_TOOLS || join(process.env.RUNNER_TEMP || tmpdir(), 'slopgate-tools-v1'));
mkdirSync(root, { recursive: true });
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
// npm.cmd is used only in this explicit provisioning step. The checker core
// never interprets command shims; runtime adapters invoke their actual entrypoint.
const installation = spawnSync(npm, ['install', '--prefix', root, '--ignore-scripts', '--no-audit', '--no-fund', '--save-exact', 'typescript@5.9.3', '@ast-grep/cli@0.45.3'], {
  stdio: 'inherit', timeout: 180_000, shell: process.platform === 'win32',
});
if (installation.error || installation.status !== 0) {
  console.error('Pinned test-tool provisioning failed.', installation.error?.message || installation.status);
  process.exit(2);
}
const platform = {
  'linux-x64': 'cli-linux-x64-gnu', 'linux-arm64': 'cli-linux-arm64-gnu',
  'darwin-x64': 'cli-darwin-x64', 'darwin-arm64': 'cli-darwin-arm64',
  'win32-x64': 'cli-win32-x64-msvc', 'win32-arm64': 'cli-win32-arm64-msvc',
}[`${process.platform}-${process.arch}`];
if (!platform) throw new Error('Unsupported test-tool platform');
const nativeDirectory = join(root, 'node_modules', '@ast-grep', platform);
if (!existsSync(join(nativeDirectory, process.platform === 'win32' ? 'ast-grep.exe' : 'ast-grep'))) throw new Error('Pinned native ast-grep binary is missing');
const paths = [nativeDirectory, join(root, 'node_modules', '.bin')];
if (process.env.GITHUB_PATH) appendFileSync(process.env.GITHUB_PATH, paths.join('\n') + '\n');
if (process.env.GITHUB_ENV) appendFileSync(process.env.GITHUB_ENV, `SLOPGATE_TEST_TOOLS=${root}\nSLOPGATE_REQUIRE_COMPILERS=1\n`);
console.log(JSON.stringify({ toolsRoot: root, prependPath: paths, versions: { typescript: '5.9.3', astGrep: '0.45.3' } }));
