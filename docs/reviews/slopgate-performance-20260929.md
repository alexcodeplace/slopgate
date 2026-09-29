# Slopgate performance review: September 29, 2026

## Trial C results, pending final qualification

A later full-pack benchmark found a repeatable approximately 5% single-file startup slowdown. Trial D is testing lazy linear prefilters before final selection; the table below is retained as Trial C evidence, not final delivery approval.

The selected Rust optimization reduces redundant matching work without changing scan policy. In the final 31-pair run, the large synthetic repository took 31.8% less median time (1.467x throughput), and the actual Slopgate Rust-source corpus took 16.4% less time (1.197x throughput). The 1,001-file corpus improved by 17.0% in latency. This is not a universal speed guarantee; small and externally dominated workloads show little or no clear improvement.

Implementation and benchmark decision: `docs/adr/0007-measured-regex-performance.md`. Delivery: PR #27. This report is a substantive self-review, not independent human approval. Exact-head CI and ordinary protected merge remain required.

## Revisions and environment

- Base: `9e3210c0dbbd18464ad4d3e3aac6c88faaa8e2a0`. The baseline binary embeds the short form `9e3210c`.
- Final measured candidate: `90e50453308a81529589f3004960b74849b28aa0`. Later evidence-only commits do not change the measured Rust implementation.
- Debian3, Intel Core i7-9700, 8 logical CPUs, Linux 7.2.6+deb14-amd64, glibc 2.43, Python 3.14.7.
- rustc 1.95.0 (59807616e 2026-04-14); locked dependencies; unchanged release LTO, one codegen unit and stripping. No target-cpu=native, architecture-specific instructions, new crates, unsafe code or result cache.
- Real tools: ast-grep 0.45.3 and TypeScript 5.9.3, provisioned before scans.
- Shared-host load was high: final one-minute averages were 12.28 before and 14.92 after, on eight cores. CPU frequency and physical disk caches were not controlled.

Binary SHA256 before: `716eb4f6e003cfbe0df78bb05d8dba4bfb0215054fcc594a4025ee59a66ac982`.

Binary SHA256 after: `e80d4badf26aa0d3957d7661cb38572e00e447a7eefac92dbee43d688bd2c843`.

Final source digest: `427f2fee35ef9caa670df82360e17a846745346488334970be0e9c17e46a995a`. Binary size changed from 4,255,152 to 4,254,640 bytes; this tiny difference is not presented as a memory optimization. Peak runtime memory was not measured.

## Final comparison

Each cell is wall-clock CLI latency, including startup and JSON output. Native rows use `scan --scope repo --tier fast`; small-file is a one-file repository, not the legacy --file command. TypeScript uses tier commit. Source corpora, policies and paths are identical for each A/B pair. Native fixtures use the no-stubs, ts-suppress and as-any packs with AST disabled. The actual-source row uses real baseline Rust files under that same benchmark policy; it is not the complete project gate with all configured external checkers.

| Workload | Before median ms | After median ms | Before p95 ms | After p95 ms | Median speedup | Paired 95% interval |
|---|---:|---:|---:|---:|---:|---:|
| small-file | 9.463 | 9.450 | 11.295 | 11.579 | 1.001x | 0.971-1.016x |
| repository-1001 | 31.852 | 26.447 | 38.266 | 32.075 | 1.204x | 1.178-1.261x |
| large-file | 23.410 | 19.115 | 27.584 | 22.310 | 1.225x | 1.201-1.307x |
| large-repository | 63.725 | 43.452 | 76.955 | 54.063 | 1.467x | 1.455-1.506x |
| large-file-positive | 26.257 | 21.584 | 37.705 | 28.776 | 1.217x | 1.162-1.248x |
| compatibility-unfilterable | 7.161 | 7.687 | 12.925 | 12.606 | 0.932x | 0.817-1.113x |
| long-line-negative | 10.402 | 9.960 | 12.581 | 13.760 | 1.044x | 0.929-1.116x |
| long-line-positive | 9.311 | 8.800 | 10.837 | 11.621 | 1.058x | 0.951-1.063x |
| slopgate-source | 28.513 | 23.823 | 38.217 | 30.533 | 1.197x | 1.170-1.278x |
| linear-dense-positive | 20.764 | 20.212 | 26.040 | 22.220 | 1.027x | 1.024-1.063x |
| compatibility-dense-positive | 20.950 | 20.991 | 24.287 | 25.966 | 0.998x | 0.972-1.030x |
| native-file-with-ast | 22.814 | 22.657 | 24.651 | 25.628 | 1.007x | 0.996-1.067x |
| typescript-incremental | 669.614 | 685.394 | 887.262 | 911.669 | 0.977x | 0.975-1.040x |

Speedup is before/after, not percent latency reduction. The point estimate is a ratio of medians; the interval bootstraps the median paired log ratio, so the two estimators need not coincide. Intervals describe these samples, not all machines or workloads. Small p95 changes and the apparent unfilterable-pattern/TypeScript slowdown are not clear repeatable regressions in this noisy run.

Corpus sizes: 1,001 files / 1,101,100 bytes; large-file 30,000 source lines / 1,207,780 bytes; large-repository 256 files / 131,072 source lines / 4,793,344 bytes; actual source corpus 73 Rust files / 742,799 bytes with 15 unchanged findings. `splitLines` in JSON includes trailing empty elements from split("\n").

## Focused noise check

After the full run, repeat the five noisy or low-benefit cases with 101 alternating pairs and five warmups per variant. Do not substitute this for the full table or omit the earlier observations.

| Workload | Before median ms | After median ms | Speedup | Paired 95% interval |
|---|---:|---:|---:|---:|
| small-file | 7.289 | 7.259 | 1.004x | 0.997-1.010x |
| compatibility-unfilterable | 4.063 | 3.867 | 1.051x | 1.021-1.049x |
| long-line-negative | 7.543 | 7.511 | 1.004x | 1.003-1.011x |
| long-line-positive | 8.102 | 8.063 | 1.005x | 1.001-1.007x |
| compatibility-dense-positive | 19.188 | 18.870 | 1.017x | 1.014-1.037x |

The previously slower unfilterable case did not reproduce as a slowdown. The TypeScript interval in the full run includes 1.0; no semantic-checker acceleration is claimed.

## Incremental experiments

- Trial A (`1109ce0`): exact case-sensitive substring checks instead of compiled literal regexes. 326 tests and 24 differential cases passed. Large-repository ratio 1.062x; source corpus 1.034x.
- Trial B (`c55e340`): compatibility-rule file prefilter and once-per-file test classification. 329 tests and 26 cases passed. Against base, large repository 1.223x and source 1.110x; against A, 1.148x and 1.088x.
- Trial C (`7d3a843`): same conservative proof for linear matchers. 330 tests and 29 cases passed. Initial 31-pair ratios 1.482x and 1.194x; against B, 1.207x and 1.073x. The final formatted implementation is the candidate in the main table.

These are separate paired runs under changing load. Raw reports preserve all observations; comparing absolute milliseconds across separate trials would be misleading.

## Correctness review

The file prefilter can reject a candidate only when a required literal is absent from the entire file. Such a literal cannot occur on any line. Presence never proves a match: anchors, flags, captures, lookarounds and backreferences are still evaluated by the original line matcher. Unsupported proofs yield no filter. The threshold of eight split lines amortizes the extra search on multiline inputs.

Case-sensitive substring matching has the same exact-literal semantics as an escaped regex. Case-insensitive literals retain the original Unicode-aware regex, including Kelvin-sign folding. Source read, UTF-8 validation, line/file bounds and rule scoping precede the file filter. Diagnostic limits, required failures, output construction and minFiles accounting are unchanged. No rule-ID shortcuts or policy changes were introduced.

The new tests exercise all shipped canaries, alternative/optional literals, scoped/global flags, Unicode, anchors, CRLF/trailing-empty lines, UTF-16 excerpts, invalid UTF-8, file/line/diagnostic limits, backtracking failure, include/exclude rules and test-file scoping. One initial property used fancy-regex as an oracle for a linear-engine rule; the unchanged baseline confirmed the engines differ on global plus scoped case folding. The property now uses the original linear matcher, and independent baseline/candidate CLI cases retain that edge case. Production behavior was not changed to satisfy the test.

## Verification evidence

- 330 workspace Rust tests: passed.
- Formatting and strict Clippy across workspace/all targets: passed.
- Architecture dependency/source/specification contract: passed.
- Existing spec-drift and hosting-policy test suites: passed.
- Seven benchmark-harness tests: passed.
- 29 real CLI acceptance tests, with required compiler tools: passed.
- Packaged npm tarball acceptance: launcher provenance, bundled self-test, non-TypeScript project rule, missing override and recursion fail-closed behavior passed.
- All 31 full-comparison workloads preserved exits and semantic reports; native cases normalize coverage timing only, external AST comparison additionally normalizes its exact scratch-directory nonce. Findings, order, coverage status, counts, paths and errors are not broadly normalized.
- Existing six release-performance budgets passed unchanged, including real AST and TypeScript; see baseline.json and budgets-trial-c.json. Those budget runs are not an interleaved A/B comparison.
- PR checks on measured source revision 90e5045: Linux, macOS, Windows, quality, architecture, workflow lint, performance and both trusted-metadata statuses passed. Final evidence-only revision requires its own checks before merge.

No finite test suite establishes the absence of every possible regression. These results establish no detected functional regression for the covered inputs, with preserved safety boundaries and passing cross-platform CI.

## Reproduction

Use separate base/candidate checkouts, the pinned toolchain, locked dependencies and identical release settings. Preserve each built binary before rebuilding. Supply the baseline source directory as common engine assets and source corpus. Build snapshots need their own .git boundary so project discovery does not select a parent repository. Standalone copied binaries need SLOPGATE_ENGINE_ROOT or the harness --engine-root option.

```sh
export SLOPGATE_TEST_TOOLS=/absolute/path/to/provisioned-tools
export PATH="$SLOPGATE_TEST_TOOLS/node_modules/@ast-grep/cli-linux-x64-gnu:$PATH"
python3 scripts/test_compare_performance.py
python3 scripts/compare_performance.py \
  --before /absolute/path/slopgate-before \
  --after /absolute/path/slopgate-after \
  --engine-root /absolute/path/base-checkout \
  --structural --semantic --samples 31 --warmups 3 \
  --output comparison.json
```

The harness downloads nothing. Provision tools using the existing scripts/provision-test-tools.mjs before measuring. Each invocation is a fresh process on a warm filesystem; only TypeScript uses its normal tool-owned incremental state. There is no Slopgate result cache.

Raw reports and logs: `docs/reviews/evidence/slopgate-performance-20260929/`. Raw logs are losslessly stored as .log.gz; `manifest.json` records stored hashes and uncompressed log hashes. Preserved worker binaries and comparison sources remain in `/home/user/benchmarks/slopgate-perf-20260929/` until disposable build artifacts are cleaned up. The original shared main checkout had unrelated edits and was not reset or stashed.
