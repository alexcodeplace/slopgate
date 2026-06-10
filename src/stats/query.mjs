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

/** @param {{ json?: boolean }} [opts] */
export function formatStats(result, { json = false } = {}) {
  if (json) return JSON.stringify(result, null, 2);
  const lines = [];
  const range = result.total ? ` (last ${result.lastSeen ?? '—'})` : '';
  lines.push(`${result.total} incident(s) stopped${range}`);
  if (result.total === 0) return lines.join('\n');

  const keyHeader = result.by.toUpperCase();
  const keyW = Math.max(keyHeader.length, ...result.groups.map((g) => String(g.key).length));
  const countW = Math.max(5, ...result.groups.map((g) => String(g.count).length));
  lines.push(`${keyHeader.padEnd(keyW)}  ${'COUNT'.padStart(countW)}  LAST SEEN`);
  for (const g of result.groups) {
    lines.push(`${String(g.key).padEnd(keyW)}  ${String(g.count).padStart(countW)}  ${g.lastSeen ?? '—'}`);
  }
  return lines.join('\n');
}
