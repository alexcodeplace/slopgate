#!/usr/bin/env python3
"""Reproducible release-CLI budgets. No daemon and no result-cache shortcuts."""
from __future__ import annotations
import argparse
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


def run(command: list[str], cwd: Path, expected: int = 0, timeout: float = 30) -> tuple[float, subprocess.CompletedProcess[str]]:
    start = time.perf_counter_ns()
    result = subprocess.run(command, cwd=cwd, text=True, encoding="utf-8", capture_output=True, timeout=timeout, check=False)
    duration = (time.perf_counter_ns() - start) / 1_000_000
    if result.returncode != expected:
        raise RuntimeError(f"benchmark check failed with {result.returncode}, expected {expected}: {result.stderr[:4000]}")
    return duration, result


def summary(samples: list[float]) -> dict:
    values = sorted(samples)
    return {"samples": len(values), "medianMs": round(statistics.median(values), 3), "p95Ms": round(values[min(len(values)-1, int(len(values)*0.95))], 3), "minMs": round(values[0], 3), "maxMs": round(values[-1], 3)}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--semantic", action="store_true")
    parser.add_argument("--structural", action="store_true")
    parser.add_argument("--baseline", type=Path, help="Prior report for explicit measured ratios; machine comparability is reported")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    budgets = json.loads((REPO / "docs/architecture/performance-budgets.json").read_text())
    failures = []
    report = {"schemaVersion": 1, "machine": {"system": platform.platform(), "architecture": platform.machine(), "logicalCpus": os.cpu_count(), "python": platform.python_version()}, "binarySha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "workloads": {}, "limits": budgets, "notes": "First observed launch is not a physical cold-disk benchmark. Semantic checker time is reported separately; no cross-run Slopgate result cache is enabled."}
    with tempfile.TemporaryDirectory(prefix="slopgate-benchmark-") as temporary:
        root = Path(temporary)
        (root / "src").mkdir()
        (root / ".slopgate").mkdir()
        config = root / ".slopgate/config.toml"
        config.write_text('roots=["src"]\nexts=[".ts"]\nastEnabled=false\nbaseline=["no-stubs","ts-suppress","as-any"]\n', encoding="utf-8")
        content = ''.join(f"export const value{i}: number = {i};\n" for i in range(32))
        source = root / "src/file.ts"
        source.write_text(content, encoding="utf-8")
        command = [str(binary), "--file", "src/file.ts", "--config", str(config)]
        initial, _ = run(command, root)
        report["firstObservedNativeLaunchMs"] = round(initial, 3)
        report["provenance"] = json.loads(run([str(binary), "capabilities"], root)[1].stdout)["engine"]
        report["workloads"]["native-file"] = summary([run(command, root)[0] for _ in range(31)])
        if args.structural:
            previous = config.read_text(encoding="utf-8")
            config.write_text(previous.replace("astEnabled=false", "astEnabled=true"), encoding="utf-8")
            structural = [str(binary), "scan", "--scope", "repo", "--tier", "fast", "--format", "json", "--config", str(config)]
            elapsed, result = run(structural, root)
            coverage = next(item for item in json.loads(result.stdout)["coverage"] if item["id"] == "ast")
            if coverage["status"] != "complete" or coverage["selectedFiles"] != 1:
                raise RuntimeError("Structural benchmark did not execute the required AST stage")
            report["structuralFirstObservedMs"] = round(elapsed, 3)
            report["astGrepVersion"] = run(["ast-grep", "--version"], root)[1].stdout.strip()
            report["workloads"]["native-file-with-ast"] = summary([run(structural, root)[0] for _ in range(21)])
            config.write_text(previous, encoding="utf-8")

        source.write_text("const padding = '" + "x" * 150_000 + "';\n", encoding="utf-8")
        report["workloads"]["long-line-negative"] = summary([run(command, root)[0] for _ in range(15)])
        source.write_text("const padding = '" + "x" * 4096 + "'; const value: Record<string, any> = {};\n", encoding="utf-8")
        report["workloads"]["long-line-positive"] = summary([run(command, root, expected=1)[0] for _ in range(15)])
        source.write_text(content, encoding="utf-8")
        for index in range(1000):
            (root / f"src/file-{index:04}.ts").write_text(content, encoding="utf-8")
        command = [str(binary), "scan", "--scope", "repo", "--tier", "fast", "--format", "json", "--config", str(config)]
        scan_samples = [run(command, root)[0] for _ in range(9)]
        report["workloads"]["native-repository"] = summary(scan_samples)
        report["workloads"]["native-repository"].update(files=1001, lines=1001*32, sourceBytes=len(content.encode())*1001, filesPerSecond=round(1001/(statistics.median(scan_samples)/1000), 1))
        if args.semantic:
            tools = Path(os.environ["SLOPGATE_TEST_TOOLS"]).resolve()
            node = shutil.which("node")
            if not node:
                raise RuntimeError("Node must be provisioned for real TypeScript measurements")
            for file in (root / "src").glob("file-*.ts"):
                file.unlink()
            package = root / "node_modules/typescript"
            package.parent.mkdir()
            shutil.copytree(tools / "node_modules/typescript", package)
            bin_dir = root / "node_modules/.bin"
            bin_dir.mkdir()
            if os.name == "nt":
                (bin_dir / "tsc.cmd").write_text('@rem package entrypoint metadata is used by the adapter\n', encoding="utf-8")
            else:
                shim = bin_dir / "tsc"
                shim.write_text(f'#!/bin/sh\nexec "{node}" "{package / "bin/tsc"}" "$@"\n', encoding="utf-8")
                shim.chmod(0o755)
            (root / "tsconfig.json").write_text(json.dumps({"compilerOptions": {"noEmit": True, "strict": True}, "include": ["src/**/*.ts"]}), encoding="utf-8")
            config.write_text('roots=["src"]\nastEnabled=false\n[checkers.tsc]\nincremental=true\n', encoding="utf-8")
            command = [str(binary), "scan", "--scope", "repo", "--format", "json", "--config", str(config)]
            cold, result = run(command, root)
            report["typescriptFirstCheckMs"] = round(cold, 3)
            adapter_samples = []
            samples = []
            for _ in range(5):
                elapsed, result = run(command, root)
                samples.append(elapsed)
                data = json.loads(result.stdout)
                adapter_samples.append(next(item["elapsedMs"] for item in data["coverage"] if item["id"] == "tsc"))
            report["workloads"]["typescript-incremental"] = summary(samples)
            report["typescriptAdapterOnly"] = summary(adapter_samples)
            report["typescriptVersion"] = run([node, str(package / "bin/tsc"), "--version"], root)[1].stdout.strip()
    for name, measured in report["workloads"].items():
        limit = budgets["workloads"][name]
        for key in ("medianMs", "p95Ms"):
            if measured[key] > limit[key]:
                failures.append(f"{name} {key}: {measured[key]} exceeds {limit[key]}")
    if args.baseline:
        previous = json.loads(args.baseline.read_text(encoding="utf-8"))
        comparable = all(previous.get("machine", {}).get(key) == report["machine"].get(key) for key in ("system", "architecture", "logicalCpus", "python"))
        ratios = {}
        for name, measured in report["workloads"].items():
            old = previous.get("workloads", {}).get(name)
            if old and all(isinstance(old.get(key), (int, float)) and old[key] > 0 for key in ("medianMs", "p95Ms")):
                ratios[name] = {key: round(measured[key] / old[key], 3) for key in ("medianMs", "p95Ms")}
        report["comparison"] = {"baselineBinarySha256": previous.get("binarySha256"), "matchingMachineMetadata": comparable, "ratios": ratios, "note": "Ratios are measured observations, not proof of identical CPU load or physical cold-cache state. Absolute checked budgets remain authoritative."}
    compiler = shutil.which("rustc")
    report["rustcVersion"] = run([compiler, "--version"], REPO)[1].stdout.strip() if compiler else "not available on benchmark host"
    report["failures"] = failures
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    if failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
