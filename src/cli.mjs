// src/cli.mjs
import { existsSync } from 'node:fs';
import { resolveConfig } from './config.mjs';
import { runGate, snapshotViolations } from './gate.mjs';
import { runSelfTest } from './selftest.mjs';
import { runInit } from './init.mjs';
import { loadBaseline, writeBaseline, writeBaselineRaw, fingerprintViolation } from './ratchet.mjs';
import { installPreCommitHook } from './install-hooks.mjs';
import { installSkills } from './install-skills.mjs';
import { installAgentHooks, removeAgentHooks, statusAgentHooks, AGENTS } from './install-agent-hooks.mjs';
import { recordIncidents } from './stats/record.mjs';
import { readRows, globalStatsPath, projectStatsPath } from './stats/store.mjs';
import { aggregate, formatStats, DIMENSIONS } from './stats/query.mjs';
import { runAudit } from './audit/audit.mjs';

const args = process.argv.slice(2);
const has = (f) => args.includes(f);
const valOf = (f) => { const i = args.indexOf(f); return i === -1 ? null : args[i + 1]; };

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

  if (has('agent-hooks')) {
    const sub = args[args.indexOf('agent-hooks') + 1];
    const validSubs = ['install', 'reinstall', 'remove', 'status'];
    const rawAgent = valOf('--agent');
    const agentIds = rawAgent ? rawAgent.split(',').map(s => s.trim()) : undefined;

    if (agentIds) {
      const unknown = agentIds.filter(id => !AGENTS.some(a => a.id === id));
      if (unknown.length) {
        process.stderr.write(`slopgate: unknown agent(s): ${unknown.join(', ')} — valid: ${AGENTS.map(a => a.id).join(', ')}\n`);
        process.exit(2);
      }
    }

    if (!sub || sub === 'status' || !validSubs.includes(sub)) {
      const rows = statusAgentHooks();
      const SYMBOL = { installed: '✓', partial: '~', 'not-installed': '✗', 'not-detected': '-', 'invalid-json': '!' };
      for (const r of rows) {
        const sym = SYMBOL[r.status] ?? '?';
        const det = r.detected ? 'detected' : 'not detected';
        process.stdout.write(`  ${sym}  ${r.label.padEnd(28)}  ${r.status.padEnd(13)}  (${det})  ${r.path}\n`);
      }
      process.exit(!sub || sub === 'status' ? 0 : 2);
    }

    if (sub === 'install' || sub === 'reinstall') {
      if (sub === 'reinstall') {
        const rem = removeAgentHooks({ agentIds });
        for (const r of rem) if (r.action === 'removed') process.stdout.write(`slopgate: agent-hooks ${r.label} — removed (reinstalling)\n`);
      }
      const results = installAgentHooks({ agentIds });
      if (results.length === 0) {
        process.stdout.write('slopgate: no agent CLIs detected — pass --agent <id> to install for a specific agent\n');
      }
      for (const r of results) {
        if (r.action === 'invalid-json') {
          process.stderr.write(`slopgate: agent-hooks ${r.label} — ${r.path} is not valid JSON, left untouched\n`);
        } else {
          process.stdout.write(`slopgate: agent-hooks ${r.label} — ${r.action} (${r.path})\n`);
        }
      }
      process.exit(0);
    }

    if (sub === 'remove') {
      const results = removeAgentHooks({ agentIds });
      for (const r of results) process.stdout.write(`slopgate: agent-hooks ${r.label} — ${r.action} (${r.path})\n`);
      process.exit(0);
    }

    process.stderr.write(`slopgate: agent-hooks usage: agent-hooks [status|install|reinstall|remove] [--agent id1,id2]\n`);
    process.exit(2);
  }

  if (has('install-skills')) {
    const force = has('--force');
    const results = installSkills({ force });
    for (const r of results) process.stdout.write(`slopgate: skill ${r.name} — ${r.action}\n`);
    if (results.length === 0) process.stdout.write('slopgate: no skills to install\n');
    process.exit(0);
  }

  if (has('audit')) {
    const config = await requireConfig();
    const sinceDays = Number(valOf('--since-days') ?? 90);
    if (!Number.isFinite(sinceDays) || sinceDays <= 0) {
      process.stderr.write('slopgate: --since-days must be a positive number\n');
      process.exit(2);
    }
    process.stdout.write(await runAudit(config, { sinceDays, json: has('--json') }) + '\n');
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
    const old = exists ? loadBaseline(config.baselinePath) : { entries: {} };
    const snap = await snapshotViolations(config);
    const n = writeBaseline(config.baselinePath, snap, new Date().toISOString());
    if (exists) {
      // show what --update amnesties: every "added" fingerprint is fresh slop being legitimized
      const fps = new Set(snap.map((v) => fingerprintViolation(v)));
      const seen = new Set();
      const added = snap.filter((v) => {
        const fp = fingerprintViolation(v);
        if (old.entries[fp] || seen.has(fp)) return false;
        seen.add(fp); return true;
      });
      const removed = Object.keys(old.entries).filter((fp) => !fps.has(fp)).length;
      const byRule = {};
      for (const v of added) byRule[v.id] = (byRule[v.id] ?? 0) + 1;
      const top = Object.entries(byRule).sort((a, b) => b[1] - a[1]).slice(0, 5)
        .map(([id, c]) => `${id}×${c}`).join(', ');
      process.stdout.write(`slopgate: baseline updated — ${n} entries (+${added.length} newly absorbed, −${removed} resolved)${top ? ` — absorbed: ${top}` : ''}\n`);
      if (added.length) process.stdout.write('slopgate: ⚠ newly absorbed entries are violations being LEGITIMIZED — review before committing baseline.json\n');
    } else {
      process.stdout.write(`slopgate: baseline written — ${n} entr${n === 1 ? 'y' : 'ies'} → ${config.baselinePath}\n`);
    }
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

  process.stderr.write('slopgate: no mode (use --staged | --file <p> | --self-test | init [dir] | baseline [--update|--prune] | install-hooks | install-skills [--force] | agent-hooks [status|install|reinstall|remove] [--agent <id>] | audit [--since-days N] [--json] | stats [--by rule|model|project|severity|engine|category] [--since <iso>] [--json] [--config <p>])\n');
  process.exit(2);
}
main().catch((e) => { process.stderr.write(`slopgate: ${e?.stack || e}\n`); process.exit(1); });
