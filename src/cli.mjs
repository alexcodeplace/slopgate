// src/cli.mjs
import { existsSync } from 'node:fs';
import { resolveConfig } from './config.mjs';
import { runGate, collectViolations, applyGateFilters } from './gate.mjs';
import { runSelfTest } from './selftest.mjs';
import { runInit } from './init.mjs';
import { loadBaseline, writeBaseline, writeBaselineRaw, fingerprintViolation } from './ratchet.mjs';
import { installPreCommitHook } from './install-hooks.mjs';
import { installSkills } from './install-skills.mjs';
import { recordIncidents } from './stats/record.mjs';
import { readRows, globalStatsPath, projectStatsPath } from './stats/store.mjs';
import { aggregate, formatStats, DIMENSIONS } from './stats/query.mjs';

const args = process.argv.slice(2);
const has = (f) => args.includes(f);
const valOf = (f) => { const i = args.indexOf(f); return i === -1 ? null : args[i + 1]; };

/** Full-repo commit-tier snapshot, filtered like the gate filters (severity + suppressions). */
async function snapshotViolations(config) {
  const { violations, notices } = await collectViolations('full', config, 'commit');
  for (const n of notices) process.stderr.write(`⚠ SLOPGATE: ${n}\n`);
  return applyGateFilters(violations, config, 'staged');
}

async function requireConfig() {
  const configPath = valOf('--config');
  if (!configPath) { process.stderr.write('slopgate: --config <path> required\n'); process.exit(2); }
  return resolveConfig(configPath);
}

async function main() {
  if (has('init')) {
    const dir = valOf('init') || process.cwd();
    process.exit(runInit(dir));
  }

  if (has('stats')) {
    const by = valOf('--by') ?? 'rule';
    if (!DIMENSIONS[by]) {
      process.stderr.write(`slopgate: --by must be ${Object.keys(DIMENSIONS).join('|')}\n`);
      process.exit(2);
    }
    const since = valOf('--since') ?? undefined;
    const json = has('--json');
    const configPath = valOf('--config');
    const rows = configPath
      ? readRows(projectStatsPath(await resolveConfig(configPath)))
      : readRows(globalStatsPath());
    process.stdout.write(formatStats(aggregate(rows, { by, since }), { json }) + '\n');
    process.exit(0);
  }

  if (has('install-hooks')) {
    const config = await requireConfig();
    const { action, path } = installPreCommitHook(config.repoRoot);
    process.stdout.write(`slopgate: pre-commit hook ${action} (${path})\n`);
    process.exit(0);
  }

  if (has('install-skills')) {
    const force = has('--force');
    const results = installSkills({ force });
    for (const r of results) process.stdout.write(`slopgate: skill ${r.name} — ${r.action}\n`);
    if (results.length === 0) process.stdout.write('slopgate: no skills to install\n');
    process.exit(0);
  }

  if (has('baseline')) {
    const config = await requireConfig();
    const exists = existsSync(config.baselinePath);

    if (has('--prune') && !has('--update')) {
      // drop entries whose fingerprint no longer occurs; never adds new ones
      const bl = loadBaseline(config.baselinePath);
      if (bl.error || bl.missing) { process.stderr.write('slopgate: no valid baseline to prune\n'); process.exit(2); }
      const current = new Set((await snapshotViolations(config)).map(fingerprintViolation));
      const kept = Object.fromEntries(Object.entries(bl.entries).filter(([fp]) => current.has(fp)));
      const dropped = Object.keys(bl.entries).length - Object.keys(kept).length;
      writeBaselineRaw(config.baselinePath, kept, new Date().toISOString());
      process.stdout.write(`slopgate: baseline pruned — ${dropped} resolved entr${dropped === 1 ? 'y' : 'ies'} removed, ${Object.keys(kept).length} kept\n`);
      process.exit(0);
    }

    if (exists && !has('--update')) {
      process.stderr.write('slopgate: baseline.json exists — use `baseline --update` to re-snapshot (this re-absorbs ALL current violations) or `baseline --prune` to drop resolved entries\n');
      process.exit(2);
    }
    const snap = await snapshotViolations(config);
    const n = writeBaseline(config.baselinePath, snap, new Date().toISOString());
    process.stdout.write(`slopgate: baseline written — ${n} entr${n === 1 ? 'y' : 'ies'} → ${config.baselinePath}\n`);
    process.exit(0);
  }

  const config = await requireConfig();
  if (has('--self-test')) process.exit(runSelfTest(config));

  const tierFlag = valOf('--tier'); // 'fast' | 'commit' | null (default by mode)
  if (tierFlag && tierFlag !== 'fast' && tierFlag !== 'commit') {
    process.stderr.write('slopgate: --tier must be fast|commit\n'); process.exit(2);
  }
  if (has('--staged')) {
    const r = await runGate('staged', config, { tier: tierFlag ?? undefined });
    if (r.code === 1) {
      try { recordIncidents(r.violations, config, { mode: 'staged' }); }
      catch (e) { process.stderr.write(`⚠ SLOPGATE: stats record failed (${e}) — ignored\n`); }
    }
    process.exit(r.code);
  }
  const fileTarget = valOf('--file');
  if (fileTarget) process.exit((await runGate('file', config, { tier: tierFlag ?? undefined, fileTarget })).code);

  process.stderr.write('slopgate: no mode (use --staged | --file <p> | --self-test | init [dir] | baseline [--update|--prune] | install-hooks | install-skills [--force] | stats [--by rule|model|project|severity|engine|category] [--since <iso>] [--json] [--config <p>])\n');
  process.exit(2);
}
main().catch((e) => { process.stderr.write(`slopgate: ${e?.stack || e}\n`); process.exit(1); });
