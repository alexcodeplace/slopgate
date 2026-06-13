#!/usr/bin/env node
// Stage-0a differential parity gate + golden generator.
//
// Proves the Rust engine produces byte-identical (normalized) output to the JS
// oracle across a violation-rich corpus, then freezes the normalized output as
// golden files for the durable Rust-vs-golden test (crates/slopgate-rs/tests/parity_golden.rs).
//
// Run modes:
//   node tools/parity/parity.mjs          # check only (fails on any divergence)
//   node tools/parity/parity.mjs --write  # check AND (re)write golden files
//
// REQUIRES the JS engine (src/cli.mjs) and ast-grep to be present. After the JS
// engine is deleted, parity is enforced solely by the Rust golden test; re-run
// this with --write only if you restore the oracle to regenerate goldens.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const REPO = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const RS_BIN = process.env.SLOPGATE_BIN || join(REPO, 'target', 'release', 'slopgate-rs');
const GOLDEN_DIR = join(REPO, 'crates', 'slopgate-rs', 'tests', 'golden');
const WRITE = process.argv.includes('--write');

const MJS = 'rules/baseline/selftest.config.mjs';
const TOML = 'rules/baseline/selftest.config.toml';

// Corpus — every item must emit violations that exercise the engine end-to-end.
// `file` items hit the real gate (sorted, deterministic output → golden).
// The `self-test` item is a pass/fail checklist whose line ORDER is cosmetic, so
// it is compared as a SET (sorted) and not frozen as golden.
const GATE = [
  { name: 'canary', file: 'rules/baseline/fixtures/src/canary.tsx' },
  { name: 'ux-ast', file: 'rules/baseline/fixtures/src/ux-ast.tsx' },
];

// Identical normalization must live in the Rust golden test. Keep in sync:
//   strip ANSI SGR sequences; strip "<n>ms" timings.
const stripAnsi = (s) => s.replace(/\x1b\[[0-9;]*m/g, '');
const stripMs = (s) => s.replace(/[0-9]+(\.[0-9]+)?ms/g, '');
const normalize = (s) => stripMs(stripAnsi(s));

function which(bin) {
  try { execFileSync('sh', ['-c', `command -v ${bin}`], { stdio: 'ignore' }); return true; }
  catch { return false; }
}

function run(cmd, args) {
  try {
    const out = execFileSync(cmd, args, { cwd: REPO, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    return { code: 0, out };
  } catch (e) {
    return { code: e.status ?? 1, out: `${e.stdout || ''}${e.stderr || ''}` };
  }
}

const fail = (msg) => { console.error(`✗ parity: ${msg}`); process.exitCode = 1; };

// ── Guards (no false-green from absent tooling) ───────────────────────────────
if (!which('ast-grep')) { fail('ast-grep MISSING — required for AST canary parity (fail loud, not skip)'); process.exit(1); }
if (!existsSync(RS_BIN)) { fail(`rust binary not built: ${RS_BIN} — run \`cargo build --release\``); process.exit(1); }
if (!existsSync(join(REPO, 'src', 'cli.mjs'))) { fail('JS oracle (src/cli.mjs) absent — cannot run differential; use the Rust golden test instead'); process.exit(1); }

let pass = 0;
if (WRITE) mkdirSync(GOLDEN_DIR, { recursive: true });

// ── self-test: SET-identical (order cosmetic), both exit 0 ────────────────────
{
  const js = run('node', ['bin/slopgate', '--self-test', '--config', MJS]);
  const rs = run(RS_BIN, ['--self-test', '--config', TOML]);
  const sortLines = (s) => normalize(s).split('\n').filter(Boolean).sort().join('\n');
  if (js.code !== 0) fail(`self-test JS exit ${js.code} (expected 0)`);
  if (rs.code !== 0) fail(`self-test Rust exit ${rs.code} (expected 0)`);
  if (sortLines(js.out) !== sortLines(rs.out)) {
    fail('self-test check SET differs JS↔Rust');
  } else pass++;
}

// ── gate items: byte-identical normalized output, both exit 1, ≥1 violation ───
for (const { name, file } of GATE) {
  const js = run('node', ['bin/slopgate', '--file', file, '--config', MJS]);
  const rs = run(RS_BIN, ['--file', file, '--config', TOML]);
  const jn = normalize(js.out);
  const rn = normalize(rs.out);
  if (js.code !== 1) fail(`${name}: JS exit ${js.code} (expected 1 — fixture must emit violations)`);
  if (!/\[(CRITICAL|HIGH)\]/.test(jn)) fail(`${name}: JS emitted no CRITICAL/HIGH violation — corpus too weak`);
  if (rs.code !== js.code) fail(`${name}: exit JS=${js.code} Rust=${rs.code}`);
  if (jn !== rn) { fail(`${name}: normalized output DIVERGES JS↔Rust`); continue; }
  pass++;
  const goldenPath = join(GOLDEN_DIR, `${name}.norm`);
  if (WRITE) { writeFileSync(goldenPath, jn); console.log(`  wrote ${goldenPath}`); }
  else if (!existsSync(goldenPath) || readFileSync(goldenPath, 'utf8') !== jn) {
    fail(`${name}: golden file stale/missing — run with --write`);
  }
}

if (process.exitCode) { console.error(`✗ parity FAILED (${pass} ok)`); }
else console.log(`✓ parity: ${pass} corpus items identical JS↔Rust${WRITE ? ' (goldens written)' : ''}`);
