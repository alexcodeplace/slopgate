// src/checkers/leakscan.mjs
/** leakscan adapter — the leaky-abstraction gate: direct DB / external-API I/O
 *  inside presentation-layer files (Dependency Inversion). Backed by a native
 *  Rust binary (tools/leakscan) that AST-parses each file and suppresses the
 *  global-call rule when the call name is locally bound — precision a regex/AST
 *  pattern can't reach. Rules/severity live in the binary; this only marshals. */
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { runJsonTool } from './shared.mjs';

const SEVERITY_MAP = { critical: 'critical', high: 'high', medium: 'medium' }; // pass-through; unknown → dropped

/** Resolve the leakscan binary: explicit cfg.bin → env → built release → built debug. */
function resolveBin(config, cfg) {
  const candidates = [
    cfg.bin ? join(config.repoRoot, cfg.bin) : null,
    process.env.LEAKSCAN_BIN || null,
    join(config.repoRoot, 'tools/leakscan/target/release/leakscan'),
    join(config.repoRoot, 'tools/leakscan/target/debug/leakscan'),
  ].filter(Boolean);
  return candidates.find(existsSync) ?? null;
}

function configFileArgs(config, cfg) {
  const p = cfg.rules
    ? join(config.configDir, cfg.rules)
    : join(config.configDir, 'leakscan.json');
  return existsSync(p) ? ['--config', p] : [];
}

export function leakscanViolations(report) {
  const out = [];
  for (const v of report.violations ?? []) {
    const severity = SEVERITY_MAP[v.severity];
    if (!severity) continue;
    if (!v.file) continue;
    out.push({
      id: `leakscan-${v.rule}`,
      severity,
      category: 'boundary',
      file: v.file,
      line: v.line ?? 1,
      fullLine: v.snippet ?? '',
      text: (v.message ?? v.rule).slice(0, 90),
      resolution: 'Route I/O through a service layer / API client — the component depends on the abstraction, not the transport.',
    });
  }
  return out;
}

export default {
  id: 'leakscan',
  detect(config, cfg) {
    if (!resolveBin(config, cfg)) {
      return { available: false, reason: 'no leakscan binary (build tools/leakscan: cargo build --release)' };
    }
    return { available: true };
  },
  async run(config, cfg) {
    const bin = resolveBin(config, cfg);
    if (!bin) return { violations: [], errors: ['no leakscan binary'] };
    const args = [...configFileArgs(config, cfg), ...config.rootsRel];
    const { data, errors } = await runJsonTool('leakscan', bin, args, {
      cwd: config.repoRoot,
      timeout: (cfg.timeout ?? 60) * 1000,
    });
    if (data == null) return { violations: [], errors };
    return { violations: leakscanViolations(data), errors: [...errors, ...(data.errors ?? [])] };
  },
};
