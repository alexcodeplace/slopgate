#!/usr/bin/env python3
"""Verify the actual npm tarball and launcher, not a source-tree-only binary."""
from __future__ import annotations
import argparse
import json
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def invoke(args: list[str], cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(args, cwd=cwd, env=env, text=True, encoding="utf-8", capture_output=True, timeout=30, check=False)
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    env = os.environ.copy()
    env.pop("SLOPGATE_BIN", None)
    env.pop("SLOPGATE_ENGINE_ROOT", None)
    env.pop("SLOPGATE_SHIM_ACTIVE", None)
    expected = invoke([str(args.binary.resolve()), "capabilities"], ROOT, env)
    if expected.returncode:
        raise RuntimeError(expected.stderr)
    digest = json.loads(expected.stdout)["engine"]["sourceDigest"]
    with tempfile.TemporaryDirectory(prefix="slopgate-package-") as temporary:
        directory = Path(temporary)
        # Invoke npm through its package CLI on Windows to avoid shell quoting.
        npm = "npm.cmd" if os.name == "nt" else "npm"
        pack = subprocess.run([npm, "pack", "--json", "--ignore-scripts", "--pack-destination", str(directory)], cwd=ROOT, env=env, capture_output=True, text=True, timeout=60, shell=os.name == "nt", check=False)
        if pack.returncode:
            raise RuntimeError(pack.stderr)
        # npm 10/11 return an array; npm 12 keys reports by package name.
        metadata = json.loads(pack.stdout)
        reports = metadata if isinstance(metadata, list) else list(metadata.values()) if isinstance(metadata, dict) else []
        if len(reports) != 1 or not isinstance(reports[0], dict) or "filename" not in reports[0]:
            raise RuntimeError("npm pack did not return exactly one verifiable package report")
        filename = reports[0]["filename"]
        if not isinstance(filename, str) or Path(filename).name != filename:
            raise RuntimeError("npm pack reported an invalid package filename")
        tarball = directory / filename
        with tarfile.open(tarball) as archive:
            names = archive.getnames()
            assert "package/bin/slopgate" in names
            assert "package/rules/baseline/selftest.config.toml" in names
            assert not any("node_modules" in name or "/.git/" in name for name in names)
            archive.extractall(directory, filter="data")
        package = directory / "package"
        launch = ["node", str(package / "bin/slopgate")]
        report = invoke([*launch, "capabilities"], package, env)
        assert report.returncode == 0, report.stdout + report.stderr
        observed = json.loads(report.stdout)
        assert observed["engine"]["sourceDigest"] == digest, "launcher selected a stale bundled binary"
        assert "vendor" in Path(observed["engine"]["executable"]).parts
        selftest = invoke([*launch, "--self-test", "--config", str(package / "rules/baseline/selftest.config.toml")], package, env)
        assert selftest.returncode == 0, selftest.stdout + selftest.stderr
        invalid = dict(env, SLOPGATE_BIN=str(directory / "nonexistent-native-binary"))
        assert invoke([*launch, "--version"], package, invalid).returncode == 2, "invalid explicit executable must not silently fall back"
        recursive = dict(env, SLOPGATE_BIN=str(package / "bin/slopgate"))
        assert invoke([*launch, "--version"], package, recursive).returncode == 2
        project = directory / "consumer"
        (project / ".slopgate").mkdir(parents=True)
        (project / "src").mkdir()
        (project / "src/app.py").write_text("BLOCK_THIS\n", encoding="utf-8")
        (project / ".slopgate/config.toml").write_text('roots=["src"]\nastEnabled=false\nrules=["rules.json"]\n', encoding="utf-8")
        (project / ".slopgate/rules.json").write_text(json.dumps({"custom": [{"id": "custom/no-block", "severity": "high", "pattern": "BLOCK_THIS", "resolution": "Remove it"}]}), encoding="utf-8")
        result = invoke([*launch, "--file", "src/app.py", "--config", ".slopgate/config.toml"], project, env)
        assert result.returncode == 1, result.stdout + result.stderr
        print(json.dumps({"package": tarball.name, "sourceDigest": digest, "launcher": "verified", "bundledSelfTest": "passed", "nonTypeScriptProjectRule": "blocked", "invalidOverride": "failed-closed"}))


if __name__ == "__main__":
    main()
