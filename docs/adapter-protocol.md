# Executable adapter protocol v1

Slopgate core is the coordinator. A language or framework integration is an executable adapter, not a new branch inside the coordinator. Built-in adapters use the same normalized finding and completion model through an in-process Rust contract. External adapters can be written in any language and communicate using one JSON request on stdin and one JSON response on stdout per batch.

## Project configuration

```toml
roots = ["src"]
exts = []
astEnabled = false
checkerConcurrency = 3

[adapters.project-policy]
executable = "python3"
args = ["tools/project_policy.py"]
scope = "project"
tier = "commit"
required = true
timeoutMs = 10000
maxOutputBytes = 8388608

[adapters.project-policy.settings]
policy = "approved-service-boundaries"
```

A bare executable is resolved through PATH. Paths containing separators are repository-relative unless absolute. Arguments are separate values: Slopgate does not insert a shell or interpolate command text. On Windows, configure an actual executable or interpreter plus a script argument, not a `.cmd` or `.bat` shim. Built-in Node adapters translate their known package command shims to the package's declared Node entrypoint.

Scopes are `files`, `project`, and `repository`. The safe default is `project`. `files` receives the selected file list and is not invoked for an empty selection. Project and repository scopes receive `files: null`, meaning they must inspect their complete configured semantic scope. Do not implement type checking by inspecting only changed files. Tiers are `fast` and `commit`, with `commit` the default. A commit scan also includes fast checks. Interactive fast-tier external adapters have a five-second execution ceiling even when their configured commit budget is longer; exceeding it is incomplete, never a clean result. Required defaults to true. Optional failures remain visible in the coverage report but do not fail the gate.

## Request

```json
{
  "protocolVersion": 1,
  "adapterId": "project-policy",
  "repoRoot": "/checkout/project",
  "scope": "project",
  "files": null,
  "mode": "full",
  "tier": "commit",
  "settings": {"policy": "approved-service-boundaries"},
  "maxConcurrency": 1
}
```

The adapter must honor its assigned concurrency budget and perform one batch rather than spawning a checker for every rule. Slopgate bounds how many adapters it launches; it cannot make arbitrary trusted adapter code cooperate internally or act as an operating-system sandbox.

## Complete response

Return exit code zero, including when policy findings exist. Slopgate, not the adapter process, decides whether findings block the gate.

```json
{
  "protocolVersion": 1,
  "status": "complete",
  "violations": [
    {
      "id": "project/no-direct-storage",
      "severity": "high",
      "category": "architecture",
      "file": "src/service.py",
      "line": 12,
      "text": "Direct storage access outside its approved boundary",
      "resolution": "Use the project-owned storage service."
    }
  ],
  "errors": []
}
```

Locations are positive, one-based line numbers in existing repository-relative source files. Absolute paths, traversal, paths escaping through symlinks, invalid severities and out-of-range locations are rejected. Slopgate owns engine provenance and rereads the actual source line, so an adapter cannot supply a forged baseline snippet. Valid severities are `critical`, `high`, `medium`, `low`, and `info`. A complete result must have no infrastructure errors.

## Incomplete response

An adapter that cannot analyze the declared scope returns `status: "error"` with explanatory errors. It must not manufacture an empty complete result. Empty output, malformed JSON, unsupported protocol versions, contradictory exit/status, crashes, output overflow, invalid findings and timeouts all produce an incomplete result. Required incomplete results exit Slopgate with code 2 and cannot be suppressed or baseline-absorbed.

Output capture defaults to 8 MiB combined stdout/stderr. Requests are limited to 8 MiB. Normalized results are bounded to 10,000 findings and 16 MiB of source excerpts, with a 64 MiB source lookup cache. These are safety limits, not permission to truncate findings and claim success. Exceeding a limit is incomplete. Tool installation is an explicit setup step, never part of scan execution.

## Rule proof and cache policy

Project JSON regex packs remain supported through `rules = ["./rules.json"]`; an intentional colliding ID requires reviewed `ruleOverrides`. Project regex rules need positive and negative canaries. Project rule self-tests additionally use `expectations.json` under the configured fixture directory. Each named case specifies its repository-relative `file` and exact `expected` findings (`id`, `engine`, `line`), and `rules` maps IDs to separate `positive` and `negative` case names. The entire finding multiset is checked through the normal file pipeline, not a relaxed fixture parser. Fixtures must remain applicable under the rule's own path globs.

V1 deliberately does not cache external adapter results. Tool-owned incremental state remains supported. Adding a result cache later requires complete source, configuration, dependency, rule, executable, environment and protocol identity. A cache keyed only by the touched file is forbidden for semantic checks.

## Security boundary

External adapters are trusted project tools, not sandboxed plugins. CI must use disposable workers, read-only repository permissions, no deployment secrets, and owner-authorized review for policy/adapter changes. ADR 0004 permits autonomous self-review after required CI. A repository administrator or a compromised reviewer credential can replace the checks themselves; code-level guards do not remove that hosting trust boundary.
