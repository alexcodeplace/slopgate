// src/stats/query.mjs
// Read-side: aggregate JSONL rows by a dimension, render table or JSON.

/** Supported group-by dimensions -> row field. 'rule' == gate-slug == row.ruleId. */
export const DIMENSIONS = {
  rule: 'ruleId',
  model: 'model',
  project: 'project',
  severity: 'severity',
  engine: 'engine',
  category: 'category',
};

/**
 * @param {object[]} rows
 * @param {{ by?: string, since?: string }} [opts]
 * @returns {{ total:number, by:string, lastSeen:string|null, firstSeen:string|null,
 *            groups:Array<{key:string,count:number,lastSeen:string|null,firstSeen:string|null}> }}
 */
export function aggregate(rows, { by = 'rule', since } = {}) {
  const field = DIMENSIONS[by];
  if (!field) throw new Error(`unknown dimension: ${by}`);
  const filtered = since ? rows.filter((r) => typeof r.ts === 'string' && r.ts >= since) : rows;

  const groups = new Map();
  let lastSeen = null, firstSeen = null;
  for (const r of filtered) {
    const key = r[field] ?? 'unknown';
    const ts = typeof r.ts === 'string' ? r.ts : null;
    let g = groups.get(key);
    if (!g) { g = { key, count: 0, lastSeen: null, firstSeen: null }; groups.set(key, g); }
    g.count += 1;
    if (ts) {
      if (!g.lastSeen || ts > g.lastSeen) g.lastSeen = ts;
      if (!g.firstSeen || ts < g.firstSeen) g.firstSeen = ts;
      if (!lastSeen || ts > lastSeen) lastSeen = ts;
      if (!firstSeen || ts < firstSeen) firstSeen = ts;
    }
  }
  const sorted = [...groups.values()].sort(
    (a, b) => b.count - a.count || String(a.key).localeCompare(String(b.key)),
  );
  return { total: filtered.length, by, lastSeen, firstSeen, groups: sorted };
}

/** Render the table body (column header + group rows) for one aggregate result.
 *  Shared by formatStats (single dimension) and formatDashboard (per section). */
function renderTable(result) {
  const keyHeader = result.by.toUpperCase();
  const keyW = Math.max(keyHeader.length, ...result.groups.map((g) => String(g.key).length));
  const countW = Math.max(5, ...result.groups.map((g) => String(g.count).length));
  const lines = [`${keyHeader.padEnd(keyW)}  ${'COUNT'.padStart(countW)}  LAST SEEN`];
  for (const g of result.groups) {
    lines.push(`${String(g.key).padEnd(keyW)}  ${String(g.count).padStart(countW)}  ${g.lastSeen ?? '—'}`);
  }
  return lines;
}

/** @param {{ json?: boolean }} [opts] */
export function formatStats(result, { json = false } = {}) {
  if (json) return JSON.stringify(result, null, 2);
  const range = result.total ? ` (last ${result.lastSeen ?? '—'})` : '';
  const lines = [`${result.total} incident(s) stopped${range}`];
  if (result.total === 0) return lines.join('\n');
  lines.push(...renderTable(result));
  return lines.join('\n');
}

/** Dimensions shown, in order, by the default `stats` dashboard. */
export const DASHBOARD_DIMS = ['rule', 'model', 'project'];

/**
 * Aggregate the same rows across every DASHBOARD_DIMS dimension.
 * total/lastSeen/firstSeen are identical across sections (same row set) — hoisted to the top.
 * @param {object[]} rows
 * @param {{ since?: string }} [opts]
 */
export function aggregateDashboard(rows, { since } = {}) {
  const sections = DASHBOARD_DIMS.map((by) => aggregate(rows, { by, since }));
  const base = sections[0];
  return { total: base.total, lastSeen: base.lastSeen, firstSeen: base.firstSeen, sections };
}

/** @param {{ json?: boolean }} [opts] */
export function formatDashboard(result, { json = false } = {}) {
  if (json) return JSON.stringify(result, null, 2);
  const range = result.total ? ` (last ${result.lastSeen ?? '—'})` : '';
  const lines = [`${result.total} incident(s) stopped${range}`];
  if (result.total === 0) return lines.join('\n');
  for (const section of result.sections) {
    lines.push('', `BY ${section.by.toUpperCase()}`, ...renderTable(section));
  }
  return lines.join('\n');
}
