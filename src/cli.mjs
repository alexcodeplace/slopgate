import { resolveConfig } from './config.mjs';
import { runGate } from './gate.mjs';
import { runSelfTest } from './selftest.mjs';
import { runInit } from './init.mjs';

const args = process.argv.slice(2);
const has = (f) => args.includes(f);
const valOf = (f) => { const i = args.indexOf(f); return i === -1 ? null : args[i + 1]; };

async function main() {
  if (has('init')) {
    const dir = valOf('init') || process.cwd();
    process.exit(runInit(dir));
  }
  const configPath = valOf('--config');
  if (!configPath) { process.stderr.write('slop-gate: --config <path> required\n'); process.exit(2); }
  const config = await resolveConfig(configPath);

  if (has('--self-test')) process.exit(runSelfTest(config));
  if (has('--staged')) process.exit(runGate('staged', config).code);
  const fileTarget = valOf('--file');
  if (fileTarget) { config._fileTarget = fileTarget; process.exit(runGate('file', config).code); }

  process.stderr.write('slop-gate: no mode (use --staged | --file <p> | --self-test | init [dir])\n');
  process.exit(2);
}
main().catch((e) => { process.stderr.write(`slop-gate: ${e?.stack || e}\n`); process.exit(1); });