// src/checkers/diff-shape.mjs
/** diff-shape — staged set spanning too many concern areas = mixed-concern commit.
 *  Concern area = configured root + first path segment under it. Staged mode only;
 *  never enters the baseline (full-mode scans skip it by design). */

export function concernGroups(files, rootsRel) {
  const groups = new Set();
  for (const f of files) {
    const root = rootsRel.find((r) => f === r || f.startsWith(`${r}/`));
    if (!root) continue;
    const rest = f.slice(root.length + 1);
    const seg = rest.includes('/') ? rest.split('/')[0] : '(root)';
    groups.add(`${root}/${seg}`);
  }
  return groups;
}

export default {
  id: 'diff-shape',
  detect() { return { available: true }; },
  run(config, cfg, { files = [], mode } = {}) {
    if (mode !== 'staged') return { violations: [], errors: [] };
    const max = cfg.maxDirs ?? 5;
    const groups = concernGroups(files, config.rootsRel);
    if (groups.size <= max) return { violations: [], errors: [] };
    return {
      violations: [{
        id: 'diff-shape-mixed-concerns', severity: 'high', category: 'hygiene',
        file: files[0], line: 1, fullLine: '',
        text: `staged files span ${groups.size} areas (max ${max})`.slice(0, 90),
        resolution: `Split into focused commits. Areas: ${[...groups].slice(0, 8).join(', ')}`,
      }],
      errors: [],
    };
  },
};
