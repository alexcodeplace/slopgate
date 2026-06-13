#!/usr/bin/env node
// Per-platform npm packaging for the native slopgate engine (esbuild/@biomejs model).
//
// The root `slopgate` package is a thin node launcher; the actual Rust binary ships
// as one prebuilt package per platform, declared in the root's optionalDependencies.
// npm installs only the package whose `os`/`cpu` match the host, so a mac user never
// downloads a linux ELF. This script is the single source of truth for that fan-out:
// it generates each platform package.json (and copies its built binary) and keeps the
// root optionalDependencies block pinned to the release version.
//
// Usage (driven by .github/workflows/release.yml, also runnable locally):
//   node scripts/build-npm-packages.mjs --only <pkg> --version <x.y.z>
//       Assemble npm/<pkg>/ : write package.json + copy target/<triple>/release/<bin>.
//       Run on the matching native CI runner after `cargo build --release --target`.
//   node scripts/build-npm-packages.mjs --sync-root --version <x.y.z>
//       Rewrite root package.json: set version + optionalDependencies pinned to it.
//   node scripts/build-npm-packages.mjs --list-matrix
//       Print the GitHub Actions matrix (JSON) for the build/publish job.
//
// No third-party deps; node >=18.

import { mkdirSync, copyFileSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..');
const REPO_URL = 'git+https://github.com/alexcodeplace/slopgate.git';

// One entry per supported host. `target` = Rust triple; `runner` = native GitHub
// runner (no cross-compilation — each binary is built on its own OS/arch).
const PLATFORMS = [
  { pkg: 'slopgate-linux-x64',    os: 'linux',  cpu: 'x64',   target: 'x86_64-unknown-linux-gnu',  exe: false, runner: 'ubuntu-latest' },
  { pkg: 'slopgate-linux-arm64',  os: 'linux',  cpu: 'arm64', target: 'aarch64-unknown-linux-gnu', exe: false, runner: 'ubuntu-24.04-arm' },
  { pkg: 'slopgate-darwin-x64',   os: 'darwin', cpu: 'x64',   target: 'x86_64-apple-darwin',       exe: false, runner: 'macos-13' },
  { pkg: 'slopgate-darwin-arm64', os: 'darwin', cpu: 'arm64', target: 'aarch64-apple-darwin',      exe: false, runner: 'macos-14' },
  { pkg: 'slopgate-win32-x64',    os: 'win32',  cpu: 'x64',   target: 'x86_64-pc-windows-msvc',     exe: true,  runner: 'windows-latest' },
];

const binName = (p) => (p.exe ? 'slopgate-rs.exe' : 'slopgate-rs');

function arg(name) {
  const i = process.argv.indexOf(name);
  return i !== -1 ? process.argv[i + 1] : undefined;
}

function requireVersion() {
  const v = arg('--version');
  if (!v || !/^\d+\.\d+\.\d+/.test(v)) {
    console.error('error: --version <x.y.z> required');
    process.exit(1);
  }
  return v;
}

// Assemble npm/<pkg>/ for a single platform: package.json + the built binary.
function buildOne(pkgName, version) {
  const p = PLATFORMS.find((x) => x.pkg === pkgName);
  if (!p) {
    console.error(`error: unknown package "${pkgName}". known: ${PLATFORMS.map((x) => x.pkg).join(', ')}`);
    process.exit(1);
  }
  const bin = binName(p);
  const src = join(REPO, 'target', p.target, 'release', bin);
  if (!existsSync(src)) {
    console.error(`error: binary not found: ${src}\n  build first: cargo build --release --target ${p.target} -p slopgate-rs`);
    process.exit(1);
  }
  const outDir = join(REPO, 'npm', p.pkg);
  mkdirSync(outDir, { recursive: true });

  const manifest = {
    name: p.pkg,
    version,
    description: `slopgate native engine — prebuilt binary for ${p.os} ${p.cpu}`,
    license: 'MIT',
    repository: { type: 'git', url: REPO_URL },
    // os/cpu let npm skip this package on non-matching hosts.
    os: [p.os],
    cpu: [p.cpu],
    files: [bin],
  };
  writeFileSync(join(outDir, 'package.json'), JSON.stringify(manifest, null, 2) + '\n');
  copyFileSync(src, join(outDir, bin));
  console.log(`built npm/${p.pkg} (v${version}, ${bin})`);
}

// Set root version + optionalDependencies (all platforms pinned exact to version).
function syncRoot(version) {
  const file = join(REPO, 'package.json');
  const json = JSON.parse(readFileSync(file, 'utf8'));
  json.version = version;
  json.optionalDependencies = Object.fromEntries(PLATFORMS.map((p) => [p.pkg, version]));
  writeFileSync(file, JSON.stringify(json, null, 2) + '\n');
  console.log(`synced root package.json -> v${version}, ${PLATFORMS.length} optionalDependencies`);
}

const cmd = process.argv[2];
if (cmd === '--only') {
  buildOne(arg('--only'), requireVersion());
} else if (cmd === '--sync-root') {
  syncRoot(requireVersion());
} else if (cmd === '--list-matrix') {
  console.log(JSON.stringify({ include: PLATFORMS }));
} else {
  console.error('usage: build-npm-packages.mjs (--only <pkg> | --sync-root | --list-matrix) [--version x.y.z]');
  process.exit(1);
}
