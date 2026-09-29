# Slopgate performance: selected results, September 29, 2026

## Delivered result

The selected optimization remains in Rust. PR #27 was reviewed and merged through the normal protected endpoint as `191a2283162bc390abbebd916cb748df1fd23893`, after all nine required checks passed. The reviewed and measured source is `fb15d2d72dbc18a972cf9b663f168debbbe87856`. This follow-up publishes selected Trial D evidence and closes the implementation ledger; it changes no executable code.

On the paired runs, the large synthetic repository improved from 58.742 to 36.987 ms median: **1.588x throughput and 37.0% less elapsed time**. The actual Rust-source corpus improved from 30.281 to 24.531 ms: **1.234x throughput and 19.0% less time**. All shipped rule packs on the larger corpus improved from 428.332 to 355.564 ms: **1.205x throughput and 17.0% less time**.

Small-file startup, dense positives and external compiler work did not show clear consistent gains. The 101-pair startup follow-up and external checks have intervals that include no change. These are shared-host measurements, not universal speed claims. No functional regression was detected in the covered tests; finite testing does not prove that every possible input or workload is regression-free.

## What changed

1. Proven case-sensitive literal checks use exact substring searches instead of separate literal regexes. Unicode-folded literals retain the original regex engine.
2. If a mandatory literal is absent from the entire validated file, the rule cannot match any line, so its per-line loop can be avoided. Positive candidates still use the original line-scoped matcher.
3. Linear-engine auxiliary proofs are lazy and only constructed when estimated line visits justify setup. This removed the roughly 5% all-pack startup penalty found in the eager Trial C implementation. Test-file classification is computed once per file.

The file filter requires at least eight split lines. Linear rules additionally require current line count times selected file count to reach 4,096 estimated visits; compatibility rules reuse proofs they already had. This heuristic chooses whether to perform an optimization, never whether to perform the actual checks. OnceLock stores a compiled proof only within the per-scan matcher, not cached results across runs.

No rule, public API, configuration, dependency, manifest, CI policy, scan limit or performance budget changed. No unsafe code, assembly, result cache or architecture-specific compiler flag was added. Decision record: `docs/adr/0007-measured-regex-performance.md`.

## Measurement method and scope

- Baseline source: `9e3210c0dbbd18464ad4d3e3aac6c88faaa8e2a0`; embedded revision is its short form `9e3210c`.
- Debian3: Intel Core i7-9700, eight logical CPUs, Linux 7.2.6+deb14-amd64, glibc 2.43; Python 3.14.7.
- Identical rustc 1.95.0 (59807616e 2026-04-14), locked dependencies and existing release profile with LTO, one codegen unit and stripping. Real ast-grep 0.45.3 and TypeScript 5.9.3 were installed before measuring.
- 31 alternating AB/BA pairs per timed workload, three warmups per variant; a separate 101-pair, five-warmup follow-up checks startup. Every launch is a fresh process with a warm filesystem. No physical cold-disk test is claimed.
- Identical paths, policy, source and engine assets for both binaries. Stage traces are separate untimed invocations. TypeScript uses only its ordinary incremental state, warmed for both variants.
- Shared load and CPU frequency were not controlled. One-minute load was 10.21 to 9.09 for the native run, 11.84 to 16.81 for startup, and 18.35 to 30.35 for the external run. Pairing reduces but cannot eliminate this uncertainty.

Default native corpora use the no-stubs, ts-suppress and as-any packs with AST disabled. The actual-source row copies 73 real baseline Rust files under that benchmark policy; it is **not** the complete production gate with all external checkers. Rows prefixed all-packs enable every shipped baseline, stack and UX pack with AST disabled. Structural and semantic work are measured separately. Small-file rows are one-file repository scans, not the legacy --file command.

Corpus sizes: repository-1001 has 1,001 files and 1,101,100 bytes; large-file has 30,000 source lines and 1,207,780 bytes; large-repository has 256 files, 131,072 source lines and 4,793,344 bytes; slopgate-source has 73 Rust files, 742,799 bytes and 15 unchanged findings. JSON splitLines includes trailing empty elements from splitting on newline.

## Selected paired comparison

All times are milliseconds, including process startup and report output. The native and external rows come from separate paired runs; compare before/after within a row, not absolute times across runs.

| Workload | Before median | After median | Before p95 | After p95 | Speedup | Paired 95% interval |
|---|---:|---:|---:|---:|---:|---:|
| small-file | 9.814 | 10.109 | 11.472 | 13.043 | 0.971x | 0.928-1.034x |
| repository-1001 | 32.249 | 26.028 | 34.383 | 32.127 | 1.239x | 1.195-1.272x |
| large-file | 23.925 | 18.316 | 28.033 | 23.570 | 1.306x | 1.290-1.348x |
| large-repository | 58.742 | 36.987 | 64.480 | 42.642 | 1.588x | 1.539-1.624x |
| all-packs-small-file | 37.395 | 37.501 | 40.300 | 43.826 | 0.997x | 0.975-1.035x |
| all-packs-repository | 428.332 | 355.564 | 446.162 | 378.564 | 1.205x | 1.185-1.211x |
| large-file-positive | 23.503 | 19.365 | 28.706 | 28.694 | 1.214x | 1.142-1.262x |
| compatibility-unfilterable | 5.976 | 5.671 | 7.465 | 6.699 | 1.054x | 0.999-1.125x |
| long-line-negative | 9.919 | 9.115 | 12.277 | 10.621 | 1.088x | 1.031-1.139x |
| long-line-positive | 10.039 | 9.878 | 12.357 | 11.177 | 1.016x | 0.987-1.130x |
| slopgate-source | 30.281 | 24.531 | 34.966 | 30.093 | 1.234x | 1.173-1.253x |
| linear-dense-positive | 19.391 | 18.938 | 21.556 | 22.867 | 1.024x | 0.992-1.027x |
| compatibility-dense-positive | 21.674 | 21.428 | 22.962 | 23.229 | 1.011x | 0.982-1.022x |
| native-file-with-ast | 31.561 | 31.432 | 41.769 | 46.046 | 1.004x | 0.873-1.019x |
| typescript-incremental | 671.721 | 678.515 | 839.157 | 787.977 | 0.990x | 0.945-1.068x |

Speedup is before/after; a 1.588x ratio is not a 58.8% latency reduction. The point estimate is a ratio of medians, while the seeded percentile bootstrap estimates the median paired log ratio. Those estimators need not coincide. Intervals describe these observations, not all machines. Small p95 increases and point estimates below 1.0 are retained in the table instead of omitted.

## Startup follow-up

The earlier eager implementation showed a repeatable all-pack startup penalty. After making auxiliary proofs lazy, the selected startup follow-up produced these 101-pair results:

| Workload | Before median | After median | Speedup | Paired 95% interval |
|---|---:|---:|---:|---:|
| small-file | 12.278 | 11.832 | 1.038x | 0.984-1.082x |
| all-packs-small-file | 43.433 | 43.843 | 0.991x | 0.987-1.018x |

Both intervals include no change. The all-pack point estimate is about 1% slower, not a demonstrated speedup; the previous clear 5% penalty did not persist. No TypeScript or AST acceleration is claimed either.

## Incremental experiments and rejected tradeoff

- Trial A (`1109ce0`): exact literal searches; modest larger-scan gains. 326 Rust tests and 24 differential cases passed.
- Trial B (`c55e340`): compatibility file prefilters and once-per-file classification. Larger synthetic corpus 1.223x and source corpus 1.110x versus base in that run. 329 tests and 26 cases passed.
- Trial C (`7d3a843`, formatted `90e5045`): eager linear prefilters. Initial larger-corpus/source ratios were 1.482x and 1.194x. A later all-pack startup test was 37.077 -> 38.864 ms, ratio 0.954x with interval 0.936-0.992x. **Not selected because of this startup regression.**
- Trial D (`a72b463`, formatted `fb15d2d`): lazy, amortized linear proof construction. Selected after 331 tests, all 33 differential cases, startup follow-up, unchanged budgets and cross-platform CI passed.

All trials are separate paired experiments under changing host load; their absolute times must not be compared as though they were one controlled run. Prior reports are retained, including C files named final-paired.json and focused-paired.json. Those names reflect the earlier experiment and are not the selected deliverable. The selected-* files identify Trial D unambiguously.

## Correctness and review

A mandatory literal absent from the whole file is absent from every line. Its presence is never treated as sufficient for a match. The original line matcher still owns anchors, flags, captures, lookarounds, backreferences and failures. Unsupported or failed proof extraction supplies no filter. Case-sensitive substring searches preserve escaped-literal semantics; Unicode folding remains unchanged, including Kelvin-sign equivalence.

Source reads, UTF-8 validation, source/line bounds and include/exclude rules run before candidate rejection. Diagnostic/backtracking limits, excerpt construction, minFiles accounting, failure policy and deterministic output are unchanged. The regression cases include scoped/global flags, optional literals, alternatives, anchors, CRLF, trailing empty lines, UTF-16 excerpts, invalid UTF-8, line/file/diagnostic bounds, true backtracking failure and test-file scoping.

One initial test used fancy-regex as oracle for an expression actually handled by the linear engine. The original baseline confirmed those engines differ with global case folding and a scoped flag removal. The oracle now uses the original linear matcher, and independent baseline/candidate CLI fixtures retain that edge case. Production behavior was not changed to satisfy the test.

Comparison ignores only coverage elapsed times and the exact six-character AST scratch-directory nonce in its project-summary metadata. Findings, order, severity, locations, rule counts, coverage status, selected files, errors, arbitrary diagnostic text and exit codes remain compared. Seven harness tests protect that boundary and the statistics/corpus identity logic.

Self-review is recorded on PR #27 at exact head fb15d2d under ADR 0004. This is not an independent human endorsement. All nine required statuses passed before the ordinary protected merge; no check was bypassed.

## Verification

- 331 workspace Rust tests: passed.
- 31 native differential cases plus two real external cases: passed with preserved outputs and exits.
- Seven comparison-harness tests: passed.
- 29 real CLI acceptance tests with required compiler tools: passed.
- Formatting, strict workspace/all-target Clippy, architecture/source/specification checks and existing governance tests: passed.
- Linux, macOS and Windows CI: passed. These jobs include real native CLI and packaged-consumer acceptance.
- Selected Linux package acceptance verifies source digest d5ddd50ef1932ef58d7fc52b110b09161874e2c38c339bcd112de674420225d3, bundled self-test, launcher selection, non-TypeScript project-rule blocking and invalid override failure.
- All six existing release performance budgets: passed unchanged.

| Existing budget workload | Selected median ms | Selected p95 ms |
|---|---:|---:|
| native-file | 9.352 | 13.148 |
| native-file-with-ast | 25.567 | 33.535 |
| long-line-negative | 8.467 | 12.176 |
| long-line-positive | 10.973 | 14.169 |
| native-repository | 25.214 | 29.852 |
| typescript-incremental | 592.369 | 613.237 |

These budget runs are not the paired comparison above. Absolute early and late timings reflect different shared-host load. Runtime peak memory, energy use and cold-disk behavior were not measured.

## Provenance and reproduction

Baseline binary SHA256: `716eb4f6e003cfbe0df78bb05d8dba4bfb0215054fcc594a4025ee59a66ac982`.

Selected binary SHA256: `ffbdb991ad374de9c210bfcbeb095fc7744d81e8765b189994edf52e07f0a1db`.

Selected source digest: `d5ddd50ef1932ef58d7fc52b110b09161874e2c38c339bcd112de674420225d3`.

Binary size changed from 4255152 to 4255904 bytes. This tiny increase is disclosed; no binary-size or runtime-memory optimization is claimed.

Build the base and candidate separately using the pinned toolchain, locked dependencies and identical existing release flags. Preserve both binaries. Source archives require a .git boundary to prevent repository discovery from selecting an unrelated parent. Standalone binaries require the supported engine-root override or harness option. Provision tools with the existing scripts/provision-test-tools.mjs before measuring. The harness itself downloads nothing.

```sh
export SLOPGATE_TEST_TOOLS=/absolute/path/to/provisioned-tools
export PATH="$SLOPGATE_TEST_TOOLS/node_modules/@ast-grep/cli-linux-x64-gnu:$PATH"
python3 scripts/test_compare_performance.py
python3 scripts/compare_performance.py \
  --before /absolute/path/slopgate-before \
  --after /absolute/path/slopgate-after \
  --engine-root /absolute/path/base-checkout \
  --samples 31 --warmups 3 --output native.json
python3 scripts/compare_performance.py \
  --before /absolute/path/slopgate-before \
  --after /absolute/path/slopgate-after \
  --engine-root /absolute/path/base-checkout \
  --structural --semantic --only native-file-with-ast typescript-incremental \
  --samples 31 --warmups 3 --output external.json
```

Raw evidence: `docs/reviews/evidence/slopgate-performance-20260929/`. Selected measurements are selected-native.json, selected-startup.json, selected-external.json and selected-budgets.json. selected-summary.json is a derived compact summary. Logs are losslessly stored as .log.gz, with stored and uncompressed hashes in manifest.json. Selected exact-head CI and implementation merge/review records are included.

The implementation worktree and its merged local/remote branch have been removed. The canonical checkout had unrelated edits before this task and was not reset, stashed or overwritten. Worker binaries and raw evidence remain under `/home/user/benchmarks/slopgate-perf-20260929/`; there is no continuing benchmark job. No npm release or version bump was made.
