// src/audit/measures.mjs
/** Pure audit computations — no fs, no git, no subprocess. All ranking math lives here
 *  so it is unit-testable without a repo. Function counts / export ratios are REGEX
 *  PROXIES (deviation from spec's ast-grep counts): ranking-grade precision, zero
 *  external tool dependency, fine for a non-gating report. */

/** LOC (non-blank) + function-count proxy. */
export function complexity(source) {
  const lines = source.split('\n');
  const loc = lines.filter((l) => l.trim()).length;
  const fnCount = (source.match(/\bfunction\b|=>/g) ?? []).length;
  return { loc, fnCount };
}

/** Hotspot rank: churn × LOC × functions (clamped ≥ 1 so plain data files still rank). */
export function hotspotScore(churn, { loc, fnCount }) {
  return churn * loc * Math.max(fnCount, 1);
}

/** Short-window churn rate ÷ long-window rate. >1 = heating up, <1 = cooling. */
export function acceleration(churnShort, shortDays, churnLong, longDays) {
  const rs = churnShort / shortDays;
  const rl = churnLong / longDays;
  if (rl === 0) return rs > 0 ? Infinity : 0;
  return rs / rl;
}

/** Exported ÷ total TOP-LEVEL decls (column-0 only). ratio ≥ 0.9 over ≥ 5 decls = shallow module. */
export function exportRatio(source) {
  const decl = /^(export\s+)?(const|let|var|function|async function|class|type|interface|enum)\b/;
  let total = 0;
  let exported = 0;
  for (const l of source.split('\n')) {
    const m = decl.exec(l);
    if (m) { total++; if (m[1]) exported++; }
  }
  return { total, exported, ratio: total ? exported / total : 0 };
}

/** Barrel = every significant line is a re-export. Pure indirection inventory. */
export function isBarrel(source) {
  const sig = source.split('\n').map((l) => l.trim())
    .filter((l) => l && !l.startsWith('//') && !l.startsWith('/*') && !l.startsWith('*'));
  return sig.length > 0 && sig.every((l) => /^export\s+(\*|\{[^}]*\})\s+from\s/.test(l));
}

/** Fan-in/fan-out per internal module from a depcruise `modules` array. */
export function fanMetrics(modules) {
  const internal = modules.filter((m) => !m.source.includes('node_modules'));
  const names = new Set(internal.map((m) => m.source));
  const fanIn = new Map();
  const rows = internal.map((m) => {
    const deps = new Set((m.dependencies ?? []).map((d) => d.resolved).filter((r) => names.has(r) && r !== m.source));
    for (const d of deps) fanIn.set(d, (fanIn.get(d) ?? 0) + 1);
    return { module: m.source, fanOut: deps.size };
  });
  return rows.map((r) => ({ ...r, fanIn: fanIn.get(r.module) ?? 0 }));
}

/**
 * Co-change pairs across per-commit file sets. Kept when:
 * shared commits ≥ minShared, shared/min(countA,countB) ≥ minRatio,
 * and groupOf(a) !== groupOf(b) (both non-null) — boundary in the wrong place.
 * Pair {a,b} is sorted lexicographically; result sorted by ratio desc, shared desc.
 */
export function coChangePairs(fileSets, groupOf, { minShared = 5, minRatio = 0.7 } = {}) {
  const fileCount = new Map();
  const pairCount = new Map();
  for (const set of fileSets) {
    const uniq = [...new Set(set)].sort();
    for (const f of uniq) fileCount.set(f, (fileCount.get(f) ?? 0) + 1);
    for (let i = 0; i < uniq.length; i++) {
      for (let j = i + 1; j < uniq.length; j++) {
        const key = `${uniq[i]} ${uniq[j]}`;
        pairCount.set(key, (pairCount.get(key) ?? 0) + 1);
      }
    }
  }
  const out = [];
  for (const [key, shared] of pairCount) {
    if (shared < minShared) continue;
    const [a, b] = key.split(' ');
    const ratio = shared / Math.min(fileCount.get(a), fileCount.get(b));
    if (ratio < minRatio) continue;
    const ga = groupOf(a);
    const gb = groupOf(b);
    if (!ga || !gb || ga === gb) continue;
    out.push({ a, b, shared, ratio });
  }
  return out.sort((x, y) => y.ratio - x.ratio || y.shared - x.shared);
}
