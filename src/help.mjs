// src/help.mjs
// Canonical help text for the slopgate CLI. Single source of truth — keep in
// sync by hand with the dispatch list in cli.mjs (one file, low churn).

export const HELP_TEXT = `slopgate — anti-slop commit gate

USAGE
  slopgate <command> [flags]

  Gate commands need --config <path> (path to a slopgate config .mjs).
  init, stats, install-skills, agent-hooks and --help do not.

COMMANDS
  --staged --config <path> [--tier fast|commit]   gate staged files (the pre-commit path)
  --file <path> --config <path> [--tier fast|commit]   gate a single file
  --self-test --config <path>                    run the rule engine self-test
  stats [--by D] [--since <iso>] [--json] [--config <path>]
                                           show blocked-incident stats; default
                                           prints a rule+model+project dashboard.
                                           --by rule|model|project|severity|engine|category
                                           narrows to one dimension.
  baseline --config <path> [--update|--prune]   manage the ratchet baseline.json
  audit --config <path> [--since-days N] [--json]   audit suppressions/baseline drift
  init [dir]                               scaffold a slopgate config (default: cwd)
  install-hooks --config <path>                    install the git pre-commit hook
  install-skills [--force]                 install bundled agent skills
  agent-hooks [status|install|reinstall|remove] [--agent <id>]
                                           manage per-agent edit/commit hooks
  --help, -h                               show this help

EXAMPLES
  slopgate stats                           # dashboard: by rule, model, project
  slopgate stats --by model --json         # one dimension as JSON
  slopgate --staged --config slopgate.config.mjs
  slopgate init`;
