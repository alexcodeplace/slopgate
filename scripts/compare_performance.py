#!/usr/bin/env python3
"""Interleaved release-CLI comparison with exact behavioral parity checks.

Use identical engine assets and provisioned tools for both binaries. This is a
warm-filesystem, fresh-process benchmark, not a cold-disk or daemon benchmark.
The existing benchmark.py remains the authority for absolute CI budgets.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import random
import re
import shutil
import statistics
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

SCAN = ["scan", "--scope", "repo", "--format", "json"]
DEFAULT_POLICY = 'roots=["src"]\nastEnabled=false\nbaseline=["no-stubs","ts-suppress","as-any"]\n'


@dataclass
class Workload:
    name: str
    root: Path
    expected: int | None = 0
    timing: bool = True
    tier: str = "fast"


def write(root: Path, relative: str, content: str | bytes) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content.encode("utf-8") if isinstance(content, str) else content)


def case(parent: Path, name: str, expected: int | None = 0, policy: str = DEFAULT_POLICY,
         timing: bool = True) -> Workload:
    root = parent / name
    write(root, ".slopgate/config.toml", policy)
    (root / "src").mkdir()
    return Workload(name, root, expected, timing)


def custom_case(parent: Path, name: str, patterns: list[dict], expected: int,
                timing: bool = False) -> Workload:
    workload = case(parent, name, expected,
                    'roots=["src"]\nastEnabled=false\nrules=["rules.json"]\n', timing)
    rules = [{"id": f"probe-{index}", "severity": "high", "resolution": "fixture",
              **pattern} for index, pattern in enumerate(patterns)]
    write(workload.root, ".slopgate/rules.json", json.dumps({"comparison": rules}))
    return workload


def make_workloads(parent: Path, source: Path) -> list[Workload]:
    workloads: list[Workload] = []
    for name, files, lines in [("small-file", 1, 32), ("repository-1001", 1001, 32),
                               ("large-file", 1, 30_000), ("large-repository", 256, 512)]:
        workload = case(parent, name)
        content = ''.join(f"export const value{i}: number = {i};\n" for i in range(lines))
        for index in range(files):
            write(workload.root, f"src/file-{index:04}.ts", content)
        workloads.append(workload)
    positive = case(parent, "large-file-positive", 1)
    write(positive.root, "src/file.ts", ''.join(f"export const value{i}: number = {i};\n" for i in range(30_000))
          + "const value: Record<string, any> = {};\n")
    workloads.append(positive)
    unfilterable = custom_case(parent, "compatibility-unfilterable", [
        {"pattern": r"(?:foo|bar)(?=!)"},
    ], 0, timing=True)
    write(unfilterable.root, "src/file.ts", "ordinary source line\n" * 30_000)
    workloads.append(unfilterable)
    for name, content, expected in [
        ("long-line-negative", "const padding = '" + "x" * 150_000 + "';\n", 0),
        ("long-line-positive", "const padding = '" + "x" * 4096 + "'; const value: Record<string, any> = {};\n", 1),
    ]:
        workload = case(parent, name, expected)
        write(workload.root, "src/file.ts", content)
        workloads.append(workload)
    real = case(parent, "slopgate-source", None)
    for directory in ("crates", "tools"):
        for file in sorted((source / directory).rglob("*.rs")):
            if "target" not in file.parts:
                write(real.root, "src/" + file.relative_to(source).as_posix(), file.read_bytes())
    workloads.append(real)
    linear_dense = custom_case(parent, "linear-dense-positive", [
        {"pattern": r"marker_[0-9]+"},
    ], 1, timing=True)
    write(linear_dense.root, "src/file.ts", "marker_42\n" * 4000)
    workloads.append(linear_dense)
    dense = custom_case(parent, "compatibility-dense-positive", [
        {"pattern": r"marker(?=_[0-9]+)"},
        {"pattern": r"(?<=prefix_)value"},
    ], 1, timing=True)
    write(dense.root, "src/file.ts", "marker_42 prefix_value\n" * 2000)
    workloads.append(dense)

    # Small differential fixtures test boundaries that throughput-only corpora miss.
    fixtures = [
        ("casefold-unicode", [{"pattern": r"k(?=elvin)", "flags": "i"}], "\u212aELVIN\nkelvin\n", 1),
        ("scoped-flags", [{"pattern": r"(?i:foo)(?=BAR)"}], "FoOBAR\nfooBAR\nfoObar\n", 1),
        ("global-scoped-flags-positive", [{"pattern": r"(?i:foo)(?-i:BAR)", "flags": "i"}], "padding\n" * 12 + "Foobar\nFOOBAR\n", 1),
        ("global-scoped-flags-negative", [{"pattern": r"(?i:foo)(?-i:BAR)", "flags": "i"}], "padding\n" * 12 + "Foobar\n", 0),
        ("lookbehind", [{"pattern": r"(?<=prefix_)value"}], "prefix_value\nvalue\nprefix_\nvalue\n", 1),
        ("alternation", [{"pattern": r"(?:foo|bar)(?=!)"}], "bar!\nfoo!\n", 1),
        ("optional-literal", [{"pattern": r"(?:optional)?value(?=!)"}], "value!\noptionalvalue!\n", 1),
        ("backreference", [{"pattern": r"(foo|bar)\1"}], "barbar\nfoofoo\nfoobar\n", 1),
        ("line-boundaries", [{"pattern": r"^value(?=!)"}], "other\r\nvalue!\r\nvalue\n!\n", 1),
        ("empty-final-line", [{"pattern": r"^(?!x)$"}], "x\n", 1),
        ("no-cross-line-match", [{"pattern": r"value(?=!)"}], "value\n!\n", 0),
        ("work-limit", [{"pattern": r"^(a+)+(?<=a)b"}], "a" * 512 + "!\n", 2),
        ("utf16-excerpt", [{"pattern": r"hit(?=!)"}], " \u05e9" + "\U0001f680" * 43 + "hit!" + "x" * 4096 + " \n", 1),
    ]
    for name, patterns, content, expected in fixtures:
        workload = custom_case(parent, name, patterns, expected)
        write(workload.root, "src/file.ts", content)
        workloads.append(workload)
    scoped = custom_case(parent, "file-scoping", [
        {"pattern": r"hit(?=!)", "minFiles": 2, "includeGlobs": ["src/**"], "excludeGlobs": ["**/excluded/**"]},
        {"pattern": r"hit(?=!)", "scanTestFiles": True, "includeGlobs": ["**/*.test.ts"]},
    ], 1)
    for file in ("src/one.ts", "src/two.ts", "src/a.test.ts", "src/excluded/a.ts", "src/__tests__/a.ts"):
        write(scoped.root, file, "hit!\n")
    workloads.append(scoped)
    for name, content in [("invalid-utf8", b"\xff\xfe"),
                          ("line-size-limit", b"x" * (1024 * 1024 + 1)),
                          ("source-size-limit", b"x\n" * (8 * 1024 * 1024 + 1))]:
        workload = case(parent, name, 2, timing=False)
        write(workload.root, "src/file.ts", content)
        workloads.append(workload)
    overflow = custom_case(parent, "diagnostic-limit", [{"pattern": r"hit(?=!)"}], 2)
    write(overflow.root, "src/file.ts", "hit!\n" * 10_001)
    workloads.append(overflow)
    return workloads


def external_workloads(parent: Path, structural: bool, semantic: bool) -> list[Workload]:
    """Provision fixtures from explicitly installed tools; never download in scans."""
    workloads = []
    content = "export const value: number = 42;\n"
    if structural:
        workload = case(parent, "native-file-with-ast", policy=DEFAULT_POLICY.replace("astEnabled=false", "astEnabled=true"))
        write(workload.root, "src/file.ts", content)
        workloads.append(workload)
    if semantic:
        tools_value = os.environ.get("SLOPGATE_TEST_TOOLS")
        node = shutil.which("node")
        if not tools_value or not node:
            raise RuntimeError("Explicit SLOPGATE_TEST_TOOLS and Node are required for semantic comparisons")
        tools = Path(tools_value).resolve(strict=True)
        workload = case(parent, "typescript-incremental", policy='roots=["src"]\nastEnabled=false\n[checkers.tsc]\nincremental=true\n')
        workload.tier = "commit"
        write(workload.root, "src/file.ts", content)
        write(workload.root, "tsconfig.json", json.dumps({"compilerOptions": {"noEmit": True, "strict": True}, "include": ["src/**/*.ts"]}))
        package = workload.root / "node_modules/typescript"
        package.parent.mkdir()
        shutil.copytree(tools / "node_modules/typescript", package)
        # Adapter discovery resolves package entrypoint metadata. On Unix retain
        # a usable shim too, with positional arguments rather than shell-quoted paths.
        if os.name == "nt":
            write(workload.root, "node_modules/.bin/tsc.cmd", "@rem package entrypoint metadata is used by the adapter\n")
        else:
            (workload.root / "node_modules/.bin").mkdir()
            (workload.root / "node_modules/.bin/tsc").symlink_to("../typescript/bin/tsc")
        workloads.append(workload)
    return workloads


def semantic_report(stdout: str) -> dict:
    """Ignore only documented nondeterministic coverage timings, not findings."""
    report = json.loads(stdout)
    if not isinstance(report, dict) or not isinstance(report.get("coverage"), list):
        raise RuntimeError("scan did not return a structured coverage report")
    for coverage in report["coverage"]:
        coverage.pop("elapsedMs", None)
    return report


def invoke(binary: Path, workload: Workload, env: dict[str, str], trace: bool = False) -> tuple[float, dict, dict]:
    current_env = dict(env)
    if trace:
        current_env["SLOPGATE_STAGE_DIAGNOSTICS"] = "1"
    else:
        current_env.pop("SLOPGATE_STAGE_DIAGNOSTICS", None)
    started = time.perf_counter_ns()
    result = subprocess.run([str(binary), *SCAN, "--tier", workload.tier, "--config", str(workload.root / ".slopgate/config.toml")],
                            cwd=workload.root, env=current_env, capture_output=True, text=True,
                            encoding="utf-8", timeout=120, check=False)
    elapsed = (time.perf_counter_ns() - started) / 1_000_000
    allowed = {0, 1} if workload.expected is None else {workload.expected}
    if result.returncode not in allowed:
        raise RuntimeError(f"{workload.name}: {binary.name} exit {result.returncode}, expected {allowed}: "
                           f"{result.stderr[:2000]} {result.stdout[:2000]}")
    stages = {}
    stderr = []
    for line in result.stderr.splitlines():
        match = re.fullmatch(r"SLOPGATE_PROGRESS stage=(\S+) event=end elapsed_ms=(\d+)", line)
        if trace and match:
            stages[match[1]] = int(match[2])
        elif not (trace and re.fullmatch(r"SLOPGATE_PROGRESS stage=\S+ event=start", line)):
            stderr.append(line)
    semantic = {"exitCode": result.returncode, "stdout": semantic_report(result.stdout), "stderr": stderr}
    return elapsed, semantic, stages


def summary(samples: list[float]) -> dict:
    values = sorted(samples)
    return {"samples": len(values), "medianMs": statistics.median(values),
            "p95Ms": values[math.ceil(0.95 * len(values)) - 1], "minMs": values[0],
            "maxMs": values[-1], "rawMs": samples}


def paired_interval(before: list[float], after: list[float]) -> list[float]:
    """Seeded percentile bootstrap of the median paired log speedup."""
    ratios = [math.log(a / b) for a, b in zip(before, after, strict=True)]
    rng = random.Random(0)
    boot = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(2000))
    return [math.exp(boot[49]), math.exp(boot[1949])]


def corpus_identity(root: Path) -> dict:
    digest = hashlib.sha256()
    source_files = source_bytes = source_lines = 0
    for file in sorted(root.rglob("*")):
        if file.is_file():
            content = file.read_bytes()
            digest.update(file.relative_to(root).as_posix().encode() + b"\0")
            digest.update(hashlib.sha256(content).digest())
            if file.is_relative_to(root / "src"):
                source_files += 1
                source_bytes += len(content)
                source_lines += content.count(b"\n") + 1
    return {"sha256": digest.hexdigest(), "files": source_files,
            "sourceBytes": source_bytes, "splitLines": source_lines}


def provenance(binary: Path, env: dict[str, str]) -> dict:
    result = subprocess.run([str(binary), "capabilities"], env=env, capture_output=True,
                            text=True, encoding="utf-8", check=True, timeout=30)
    return {"binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "binaryBytes": binary.stat().st_size, "engine": json.loads(result.stdout)["engine"]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", required=True, type=Path)
    parser.add_argument("--after", required=True, type=Path)
    parser.add_argument("--engine-root", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--samples", type=int, default=21)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--structural", action="store_true")
    parser.add_argument("--semantic", action="store_true")
    parser.add_argument("--only", nargs="*", help="Select workload names for an exploratory run")
    args = parser.parse_args()
    if args.samples < 3 or args.warmups < 0:
        parser.error("samples must be at least 3 and warmups must be nonnegative")
    before, after = args.before.resolve(strict=True), args.after.resolve(strict=True)
    source = args.engine_root.resolve(strict=True)
    env = dict(os.environ, SLOPGATE_ENGINE_ROOT=str(source))
    report = {"schemaVersion": 1, "machine": {"platform": platform.platform(),
              "cpu": platform.processor(), "logicalCpus": os.cpu_count(),
              "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
              "python": platform.python_version(), "loadBefore": os.getloadavg() if hasattr(os, "getloadavg") else None},
              "before": provenance(before, env), "after": provenance(after, env),
              "method": {"order": "alternating AB/BA pairs", "warmupsPerVariant": args.warmups,
                         "samplesPerVariant": args.samples, "normalization": "coverage[].elapsedMs only",
                         "notes": "Fresh process, warm filesystem; no Slopgate result cache. TypeScript uses shared tool-owned incremental state warmed for both variants. Shared host load and CPU frequency are not controlled. Stage traces are untimed additional runs. Bootstrap intervals describe these paired samples, not other machines."},
              "workloads": {}, "failures": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with tempfile.TemporaryDirectory(prefix="slopgate-comparison-") as temporary:
            workloads = make_workloads(Path(temporary), source)
            workloads.extend(external_workloads(Path(temporary), args.structural, args.semantic))
            if args.only:
                known = {workload.name for workload in workloads}
                unknown = set(args.only) - known
                if unknown:
                    raise ValueError(f"unknown workloads: {sorted(unknown)}")
                workloads = [workload for workload in workloads if workload.name in args.only]
            for workload in workloads:
                entry = {"corpus": corpus_identity(workload.root), "behaviorIdentical": False}
                report["workloads"][workload.name] = entry
                _, oracle, stages = invoke(before, workload, env, trace=True)
                entry["stageTraceBeforeMs"] = stages
                _, checked, stages = invoke(after, workload, env, trace=True)
                entry["stageTraceAfterMs"] = stages
                if checked != oracle:
                    entry["behaviorDifference"] = {"before": oracle, "after": checked}
                    raise RuntimeError(f"behavior changed in {workload.name}")
                if workload.name in {"native-file-with-ast", "typescript-incremental"}:
                    stage = "ast" if workload.name == "native-file-with-ast" else "tsc"
                    coverage = next((item for item in oracle["stdout"]["coverage"] if item["id"] == stage), None)
                    if not coverage or coverage["status"] != "complete":
                        raise RuntimeError(f"{workload.name}: required external stage did not complete")
                entry.update(behaviorIdentical=True, exitCode=oracle["exitCode"],
                             findings=len(oracle["stdout"]["violations"]),
                             semanticSha256=hashlib.sha256(json.dumps(oracle, sort_keys=True).encode()).hexdigest())
                if not workload.timing:
                    continue
                samples = {"before": [], "after": []}
                for round_index in range(args.warmups + args.samples):
                    order = [("before", before), ("after", after)]
                    if round_index % 2:
                        order.reverse()
                    for label, binary in order:
                        elapsed, result, _ = invoke(binary, workload, env)
                        if result != oracle:
                            raise RuntimeError(f"nondeterministic or changed output: {workload.name}, {label}")
                        if round_index >= args.warmups:
                            samples[label].append(elapsed)
                entry["before"] = summary(samples["before"])
                entry["after"] = summary(samples["after"])
                entry["medianSpeedup"] = entry["before"]["medianMs"] / entry["after"]["medianMs"]
                entry["pairedSpeedup95Pct"] = paired_interval(samples["before"], samples["after"])
                print(f"{workload.name}: {entry['before']['medianMs']:.3f} -> "
                      f"{entry['after']['medianMs']:.3f} ms ({entry['medianSpeedup']:.3f}x), exact parity", flush=True)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report["failures"].append(str(error))
        raise
    finally:
        report["machine"]["loadAfter"] = os.getloadavg() if hasattr(os, "getloadavg") else None
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
