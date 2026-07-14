#!/usr/bin/env node
// Single-package npm distribution for the native slopgate engine.
//
// One package `slopgate` ships every platform's prebuilt Rust binary under
// vendor/<platform>-<arch>/. npm delivers them all; bin/slopgate picks the one
// matching the host at runtime. No per-platform packages, no optionalDependencies.
//
// Because each binary is built natively (no cross-compilation), CI builds the
// five targets on five runners, each staging its binary as an artifact; a final
// job collects them all into vendor/ and publishes the single package.
//
// Usage (driven by .github/workflows/release.yml, also runnable locally):
//   node scripts/build-npm-packages.mjs --stage <key>
//       Copy target/<triple>/release/<bin> -> vendor/<key>/<bin> for one host.
//       Run on the matching native runner after `cargo build --release --target`.
//   node scripts/build-npm-packages.mjs --set-version <x.y.z>
//       Rewrite root package.json version (called once before publish).
//   node scripts/build-npm-packages.mjs --list-matrix
//       Print the GitHub Actions build matrix (JSON).
//
// No third-party deps; node >=18.

import { mkdirSync, copyFileSync, writeFileSync, readFileSync, existsSync, chmodSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..');

// One entry per supported host. `key` = vendor dir = node `<platform>-<arch>`
// (matches bin/slopgate's lookup). `target` = Rust triple; `runner` = native
// GitHub runner (no cross-compilation — each binary built on its own OS/arch).
const PLATFORMS = [
  { key: 'linux-x64',    target: 'x86_64-unknown-linux-gnu',  exe: false, runner: 'ubuntu-latest' },
  { key: 'linux-arm64',  target: 'aarch64-unknown-linux-gnu', exe: false, runner: 'ubuntu-24.04-arm' },
  { key: 'darwin-x64',   target: 'x86_64-apple-darwin',       exe: false, runner: 'macos-14' },
  { key: 'darwin-arm64', target: 'aarch64-apple-darwin',      exe: false, runner: 'macos-14' },
  { key: 'win32-x64',    target: 'x86_64-pc-windows-msvc',    exe: true,  runner: 'windows-latest' },
];

const binName = (p) => (p.exe ? 'slopgate-rs.exe' : 'slopgate-rs');

function arg(name) {
  const i = process.argv.indexOf(name);
  return i !== -1 ? process.argv[i + 1] : undefined;
}

// Copy one freshly-built target binary into vendor/<key>/.
function stage(key) {
  const p = PLATFORMS.find((x) => x.key === key);
  if (!p) {
    console.error(`error: unknown platform "${key}". known: ${PLATFORMS.map((x) => x.key).join(', ')}`);
    process.exit(1);
  }
  const bin = binName(p);
  const src = join(REPO, 'target', p.target, 'release', bin);
  if (!existsSync(src)) {
    console.error(`error: binary not found: ${src}\n  build first: cargo build --release --target ${p.target} -p slopgate-rs`);
    process.exit(1);
  }
  const outDir = join(REPO, 'vendor', p.key);
  mkdirSync(outDir, { recursive: true });
  const dest = join(outDir, bin);
  copyFileSync(src, dest);
  if (!p.exe) chmodSync(dest, 0o755);
  console.log(`staged vendor/${p.key}/${bin}`);
}

function smoke(key) {
  const p = PLATFORMS.find((x) => x.key === key);
  if (!p) {
    console.error(`error: unknown platform "${key}". known: ${PLATFORMS.map((x) => x.key).join(', ')}`);
    process.exit(1);
  }
  const bin = join(REPO, 'vendor', p.key, binName(p));
  if (!existsSync(bin)) {
    console.error(`error: staged binary not found: ${bin}`);
    process.exit(1);
  }
  const result = spawnSync(bin, ['--version'], { stdio: 'inherit' });
  if (result.error) {
    console.error(`error: staged binary failed to execute: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

function setVersion(version) {
  if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
    console.error('error: --set-version <x.y.z> required');
    process.exit(1);
  }
  const file = join(REPO, 'package.json');
  const json = JSON.parse(readFileSync(file, 'utf8'));
  json.version = version;
  writeFileSync(file, JSON.stringify(json, null, 2) + '\n');
  console.log(`set root package.json version -> ${version}`);
}

const cmd = process.argv[2];
if (cmd === '--stage') {
  stage(arg('--stage'));
} else if (cmd === '--set-version') {
  setVersion(arg('--set-version'));
} else if (cmd === '--list-matrix') {
  console.log(JSON.stringify({ include: PLATFORMS }));
} else if (cmd === '--smoke') {
  smoke(arg('--smoke'));
} else {
  console.error('usage: build-npm-packages.mjs (--stage <key> | --smoke <key> | --set-version <x.y.z> | --list-matrix)');
  process.exit(1);
}
