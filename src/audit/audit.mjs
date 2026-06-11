// src/audit/audit.mjs
/** Audit orchestrator + renderer — assembles architecture-health sections from git facts,
 *  depcruise graph, ratchet baseline, suppressions, and gate stats. Never throws. */
import { readFileSync, existsSync } from 'node:fs';
import { basename, join, relative } from 'node:path';
import { complexity, hotspotScore, acceleration, exportRatio, isBarrel, fanMetrics, coChangePairs } from './measures.mjs';
import { churnByFile, commitFileSets, authorShares, fileAtDaysAgo, jsonEntryHistory } from './git-facts.mjs';
import { listSourceFiles } from '../enumerate.mjs';
import { loadBaseline } from '../ratchet.mjs';
import { loadSuppressions, pruneStale } from '../suppressions.mjs';
import { snapshotViolations } from '../gate.mjs';
import { runDepcruiseJson } from '../checkers/depcruise.mjs';
import { ensureCacheDir } from '../checkers/shared.mjs';
import { readRows, projectStatsPath } from '../stats/store.mjs';
import { aggregate } from '../stats/query.mjs';

const SHORT_DAYS = 30;

/** Concern area = configured root + first path segment (matches diff-shape). */
function concernArea(file, rootsRel) {
  const root = rootsRel.find((r) => file === r || file.startsWith(`${r}/`));
  if (!root) return null;
  const rest = file.slice(root.length + 1);
  const seg = rest.includes('/') ? rest.split('/')[0] : '(root)';
  return `${root}/${seg}`;
}

function concernAreas(rootsRel) {
  return rootsRel.map((r) => `${r}/(root)`);
}

function hasTestFile(repoRoot, file) {
  const base = file.replace(/\.(tsx?|astro)$/, '');
  return ['.test.ts', '.test.tsx'].some((ext) => existsSync(join(repoRoot, base + ext)));
}

/** @returns {{ rows: Array<{ file, churn, loc, fnCount, score, accel, untested }> }} */
export function buildHotspots(sources, churn90, churn30, { sinceDays, oldSourceOf, hasTest }) {
  const rows = [];
  for (const [file, source] of sources) {
    const churnLong = churn90.get(file) ?? 0;
    if (churnLong === 0) continue;
    const churnShort = churn30.get(file) ?? 0;
    const comp = complexity(source);
    rows.push({
      file,
      churn: churnLong,
      loc: comp.loc,
      fnCount: comp.fnCount,
      score: hotspotScore(churnLong, comp),
      accel: acceleration(churnShort, SHORT_DAYS, churnLong, sinceDays),
      untested: !hasTest(file),
      locDelta: null,
    });
  }
  rows.sort((a, b) => b.score - a.score);
  const top = rows.slice(0, 10);
  if (oldSourceOf) {
    for (const row of top) {
      const old = oldSourceOf(row.file);
      row.locDelta = old ? row.loc - complexity(old).loc : null;
    }
  }
  return { rows: top };
}

/** Flags when top author share ≥ 0.8 AND commits ≥ 5. */
export function buildKnowledge(dirShares, { shareThreshold = 0.8, minCommits = 5 } = {}) {
  const rows = dirShares.map(({ dir, shares }) => {
    const top = shares[0];
    const total = shares.reduce((acc, s) => acc + s.commits, 0);
    const flagged = !!(top && top.share >= shareThreshold && total >= minCommits);
    return {
      dir,
      topAuthor: top?.author ?? null,
      topShare: top?.share ?? 0,
      topCommits: top?.commits ?? 0,
      flagged,
    };
  });
  return { rows };
}

/** etaDays null when < 2 points, flat, or not decreasing. */
export function buildBurndown(history) {
  const hist = history ?? [];
  if (hist.length < 2) return { history: hist, perDay: 0, etaDays: null };
  const first = hist[0];
  const last = hist[hist.length - 1];
  const ms = new Date(last.ts).getTime() - new Date(first.ts).getTime();
  const days = ms / (1000 * 60 * 60 * 24);
  const delta = first.count - last.count;
  const perDay = days > 0 ? delta / days : 0;
  let etaDays = null;
  if (delta > 0 && perDay > 0 && last.count > 0) etaDays = last.count / perDay;
  return { history: hist, perDay, etaDays };
}

export function renderAudit({ header, sections, notices }) {
  const lines = [header, ''];
  for (const s of sections) {
    lines.push(`== ${s.title} ==`);
    if (s.lines?.length) lines.push(...s.lines);
    else lines.push('(nothing to report)');
    lines.push('');
  }
  if (notices?.length) {
    lines.push('-- skipped --');
    lines.push(...notices);
  }
  return lines.join('\n');
}

function fmtAccel(v) {
  if (v === Infinity) return '∞';
  if (!Number.isFinite(v)) return String(v);
  return v.toFixed(2);
}

function hotspotLines(hs) {
  return hs.rows.map((r, i) => {
    const extra = [r.untested ? 'untested' : null, r.locDelta != null && r.locDelta > 0 ? `+${r.locDelta} loc` : null]
      .filter(Boolean).join('  ');
    const tail = extra ? `  ${extra}` : '';
    return `${i + 1}. ${r.file}  churn=${r.churn} loc=${r.loc} score=${Math.round(r.score)} accel=${fmtAccel(r.accel)}${tail}`;
  });
}

function moduleShapeLines(sources, modules) {
  const lines = [];
  const fans = fanMetrics(modules ?? []);

  for (const [file, source] of sources) {
    const er = exportRatio(source);
    if (er.total >= 5 && er.ratio >= 0.9) {
      lines.push(`shallow export surface: ${file} (${er.exported}/${er.total} exported)`);
    }
    if (isBarrel(source)) lines.push(`barrel: ${file}`);
  }

  for (const f of fans) {
    if (f.fanOut >= 8) lines.push(`fan-out god: ${f.module} (out=${f.fanOut})`);
    if (f.fanIn === 1 && f.fanOut > 0) lines.push(`single-consumer: ${f.module} (in=1 out=${f.fanOut})`);
  }

  return lines.slice(0, 20);
}

function coChangeLines(pairs) {
  return pairs.slice(0, 15).map((p, i) =>
    `${i + 1}. ${p.a} ↔ ${p.b}  shared=${p.shared} ratio=${p.ratio.toFixed(2)}`,
  );
}

function knowledgeLines(k) {
  return k.rows
    .filter((r) => r.topAuthor)
    .sort((a, b) => b.topShare - a.topShare)
    .map((r) => {
      const flag = r.flagged ? '  ⚠ concentrated' : '';
      return `${r.dir}: ${r.topAuthor} ${(r.topShare * 100).toFixed(0)}% (${r.topCommits} commits)${flag}`;
    });
}

function ratchetLines(bl, currentCount, burndown) {
  const lines = [];
  const baselineCount = Object.keys(bl.entries).length;
  lines.push(`baseline entries: ${baselineCount}`);
  lines.push(`current violations (filtered): ${currentCount}`);
  if (currentCount < baselineCount) lines.push(`net progress: ${baselineCount - currentCount} resolved since baseline snapshot`);
  else if (currentCount > baselineCount) lines.push(`regression: +${currentCount - baselineCount} since baseline snapshot`);
  if (burndown.history.length >= 2) {
    const eta = burndown.etaDays != null ? `~${Math.round(burndown.etaDays)} days` : 'n/a';
    lines.push(`burn-down: ${burndown.perDay.toFixed(2)} entries/day  ETA ${eta}`);
  }
  return lines;
}

function exemptionLines(sup, pruned, health, config) {
  const lines = [];
  if (sup.error) lines.push(`suppressions.json malformed: ${sup.error}`);
  else lines.push(`active suppressions: ${sup.entries.length}`);
  if (pruned.pruned.length) lines.push(`stale suppressions (dry-run): ${pruned.pruned.length}`);
  if (config?.astDisable?.size > 0) {
    lines.push(`astDisable exemptions: ${[...config.astDisable].join(', ')} — still justified?`);
  }
  if (health) {
    for (const [id, st] of Object.entries(health.checkers ?? {})) {
      if (st.consecutiveFailures >= 2) {
        lines.push(`CHECKER OFF: ${id} (${st.consecutiveFailures} consecutive infra failures)`);
      }
    }
  }
  return lines;
}

function gateEffectivenessLines(stats) {
  if (!stats.total) return [];
  const lines = [`${stats.total} incident(s) stopped (gate effectiveness)`];
  for (const g of stats.groups.slice(0, 10)) {
    lines.push(`  ${g.key}: ${g.count}`);
  }
  return lines;
}

/**
 * @param {import('../config.mjs').ResolvedConfig} config
 * @param {{ sinceDays?: number, json?: boolean }} [opts]
 * @returns {Promise<string>}
 */
export async function runAudit(config, { sinceDays = 90, json = false } = {}) {
  const notices = [];
  const sections = [];
  const project = basename(config.repoRoot);
  const header = `SLOPGATE AUDIT — ${project} — window ${sinceDays}d`;

  try {
    const files = listSourceFiles(config);
    const sources = new Map();
    for (const f of files) {
      try { sources.set(f, readFileSync(join(config.repoRoot, f), 'utf8')); }
      catch { /* unreadable — skip */ }
    }

    // Hotspots (git)
    try {
      const churn90 = churnByFile(config.repoRoot, sinceDays);
      const churn30 = churnByFile(config.repoRoot, SHORT_DAYS);
      if (churn90.size === 0) notices.push('hotspots skipped (no git history)');
      else {
        const hs = buildHotspots(sources, churn90, churn30, {
          sinceDays,
          oldSourceOf: (f) => fileAtDaysAgo(config.repoRoot, f, sinceDays),
          hasTest: (f) => hasTestFile(config.repoRoot, f),
        });
        sections.push({ title: 'Hotspots (churn x size)', lines: hotspotLines(hs) });
      }
    } catch (e) { notices.push(`hotspots skipped (${e})`); }

    // Module shape (depcruise)
    try {
      const depCfg = config.checkers?.depcruise;
      if (!depCfg) {
        notices.push('module shape skipped (no depcruise)');
      } else {
        const { data, errors } = await runDepcruiseJson(config, depCfg);
        if (data?.modules) {
          sections.push({ title: 'Module shape', lines: moduleShapeLines(sources, data.modules) });
        } else {
          notices.push(`module shape skipped (${errors[0] ?? 'no depcruise output'})`);
        }
      }
    } catch (e) { notices.push(`module shape skipped (${e})`); }

    // Co-change coupling (git)
    try {
      const sets = commitFileSets(config.repoRoot, sinceDays);
      if (sets.length === 0) notices.push('co-change skipped (no git history)');
      else {
        const groupOf = (f) => concernArea(f, config.rootsRel);
        const pairs = coChangePairs(sets, groupOf);
        sections.push({ title: 'Co-change coupling', lines: coChangeLines(pairs) });
      }
    } catch (e) { notices.push(`co-change skipped (${e})`); }

    // Knowledge concentration (git; empty rows if no git)
    try {
      const areas = new Set();
      for (const f of files) {
        const a = concernArea(f, config.rootsRel);
        if (a) areas.add(a);
      }
      for (const a of concernAreas(config.rootsRel)) areas.add(a);
      const dirShares = [...areas].sort().map((dir) => ({
        dir,
        shares: authorShares(config.repoRoot, sinceDays, dir),
      }));
      const k = buildKnowledge(dirShares);
      sections.push({ title: 'Knowledge concentration', lines: knowledgeLines(k) });
    } catch (e) { notices.push(`knowledge skipped (${e})`); }

    // Ratchet progress + burn-down
    try {
      const bl = loadBaseline(config.baselinePath);
      if (bl.missing) notices.push('ratchet skipped (no baseline.json)');
      else if (bl.error) notices.push(`ratchet skipped (baseline malformed: ${bl.error})`);
      else {
        const relBaseline = relative(config.repoRoot, config.baselinePath);
        const hist = jsonEntryHistory(config.repoRoot, relBaseline);
        const burndown = buildBurndown(hist);
        let currentCount = 0;
        try {
          const violations = await snapshotViolations(config);
          currentCount = violations.length;
        } catch (e) { notices.push(`ratchet current count skipped (${e})`); }
        sections.push({ title: 'Ratchet progress + burn-down', lines: ratchetLines(bl, currentCount, burndown) });
      }
    } catch (e) { notices.push(`ratchet skipped (${e})`); }

    // Exemptions & checker health
    try {
      const sup = loadSuppressions(config.suppressionsPath);
      const pruned = pruneStale(config.repoRoot, config.suppressionsPath, { dryRun: true });
      let health = null;
      const healthPath = join(ensureCacheDir(config), 'checker-health.json');
      if (existsSync(healthPath)) {
        try { health = JSON.parse(readFileSync(healthPath, 'utf8')); } catch { /* malformed */ }
      }
      sections.push({ title: 'Exemptions & checker health', lines: exemptionLines(sup, pruned, health, config) });
    } catch (e) { notices.push(`exemptions skipped (${e})`); }

    // Gate effectiveness (stats.jsonl)
    try {
      const rows = readRows(projectStatsPath(config));
      if (rows.length === 0) notices.push('gate effectiveness skipped (no stats.jsonl)');
      else {
        const stats = aggregate(rows);
        sections.push({ title: 'Gate effectiveness', lines: gateEffectivenessLines(stats) });
      }
    } catch (e) { notices.push(`gate effectiveness skipped (${e})`); }
  } catch (e) {
    notices.push(`audit error: ${e}`);
  }

  const report = { header, sections, notices };
  return json ? JSON.stringify(report) : renderAudit(report);
}
