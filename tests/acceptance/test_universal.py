"""Real CLI/process acceptance for universal Slopgate (SG-TEST-001).

Run with Python 3.11+: python tests/acceptance/test_universal.py --binary PATH.
Compiler tests require explicitly provisioned test tools; no scan installs them.
"""
from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("SLOPGATE_TEST_BINARY", REPOSITORY / "target/debug/slopgate-rs")).resolve()
TOOL_ROOT = Path(os.environ["SLOPGATE_TEST_TOOLS"]).resolve() if os.environ.get("SLOPGATE_TEST_TOOLS") else None
REQUIRE_COMPILERS = os.environ.get("SLOPGATE_REQUIRE_COMPILERS") == "1"


def execute(args: list[str], cwd: Path, timeout: float = 20.0) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "SLOPGATE_PROGRESS"):
        env.pop(key, None)
    env["PYTHONIOENCODING"] = "utf-8"
    return subprocess.run(args, cwd=cwd, env=env, text=True, encoding="utf-8", capture_output=True, timeout=timeout, check=False)


class GateAcceptance(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="slopgate-universal-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.root = self.base / "repo"
        self.root.mkdir()
        (self.root / "src").mkdir()
        (self.root / ".slopgate").mkdir()
        self.git("init", "--quiet")
        self.git("config", "user.email", "acceptance@example.invalid")
        self.git("config", "user.name", "Slopgate acceptance fixture")
        self.write("src/source.data", "clean source\n")
        self.config("")

    def write(self, relative: str, contents: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding="utf-8")
        return path

    def config(self, extra: str = "", *, native_ast: bool = False, roots: list[str] | None = None) -> None:
        self.write(".slopgate/config.toml", f"roots = {json.dumps(roots or ['src'])}\nexts = []\nastEnabled = {str(native_ast).lower()}\n" + extra)

    def git(self, *args: str) -> str:
        result = execute(["git", *args], self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def gate(self, *args: str, expected: int, timeout: float = 20) -> subprocess.CompletedProcess[str]:
        result = execute([str(BINARY), *args, "--config", str(self.root / ".slopgate/config.toml")], self.root, timeout)
        self.assertEqual(result.returncode, expected, f"args={args}\nstdout={result.stdout}\nstderr={result.stderr}")
        return result

    def scan(self, expected: int, *, tier: str = "commit", timeout: float = 20) -> dict:
        result = self.gate("scan", "--scope", "repo", "--tier", tier, "--format", "json", expected=expected, timeout=timeout)
        data = json.loads(result.stdout)
        self.assertEqual(data["exitCode"], expected)
        self.assertEqual(data["infraFailed"], expected == 2)
        return data

    def add_regex(self, pattern: str = "FORBIDDEN_TOKEN", identifier: str = "project/no-slop") -> None:
        self.write(".slopgate/rules.json", json.dumps({"project": [{"id": identifier, "severity": "high", "pattern": pattern, "resolution": "Remove the forbidden content.", "canary": "FORBIDDEN_TOKEN", "negativeCanary": ["clean"], "scanTestFiles": True}]}))
        self.config('rules = ["./rules.json"]\n')

    def adapter(self, behavior: str = "pass", *, required: bool = True, scope: str = "project", tier: str = "commit", timeout_ms: int = 5000, **settings: object) -> None:
        shutil.copyfile(REPOSITORY / "tests/acceptance/protocol_adapter.py", self.root / "adapter.py")
        self.config('[adapters.example]\n' + f'executable = {json.dumps(sys.executable)}\nargs = ["adapter.py"]\n' + f'required = {str(required).lower()}\nscope = "{scope}"\ntier = "{tier}"\ntimeoutMs = {timeout_ms}\nmaxOutputBytes = 65536\n' + '[adapters.example.settings]\n' + '\n'.join(f'{key} = {json.dumps(value, ensure_ascii=False)}' for key, value in {"behavior": behavior, **settings}.items()) + '\n')

    def test_regex_all_languages_and_test_files_have_same_gate_contract(self) -> None:
        self.add_regex()
        for name in ("app.ts", "view.tsx", "module.mts", "module.cts", "app.py", "app.rs", "page.astro", "script.sh", "unknown.language", "regression.test.rs"):
            with self.subTest(name=name):
                self.write(f"src/{name}", "FORBIDDEN_TOKEN\n")
                self.gate("--file", f"src/{name}", expected=1)
        data = self.scan(1, tier="fast")
        findings = [finding for finding in data["violations"] if finding["id"] == "project/no-slop"]
        self.assertEqual(len(findings), 10)
        self.assertTrue(all(finding["line"] == 1 for finding in findings))

    def test_required_protocol_failure_never_becomes_clean(self) -> None:
        for behavior in ("error", "malformed", "empty", "version", "field", "contradict", "nonzero", "traversal", "location", "severity"):
            with self.subTest(behavior=behavior):
                self.adapter(behavior)
                data = self.scan(2)
                self.assertTrue(data["errors"])
                self.assertEqual(next(c for c in data["coverage"] if c["id"] == "example")["status"], "error")

    @unittest.skipIf(os.name == "nt", "POSIX named-pipe source validation")
    def test_protocol_cannot_block_on_a_named_pipe_source(self) -> None:
        self.adapter("fail")
        source = self.root / "src/source.data"
        source.unlink()
        os.mkfifo(source)
        # Keep the pipe outside discovery so this reaches response normalization
        # rather than merely testing the source enumerator's existing safeguard.
        self.write("safe/clean.data", "clean\n")
        config = self.root / ".slopgate/config.toml"
        config.write_text(config.read_text().replace('["src"]', '["safe"]'))
        self.scan(2, timeout=5)

    def test_protocol_finding_is_blocking_and_provenance_is_owned_by_gate(self) -> None:
        self.adapter("fail")
        data = self.scan(1)
        finding = data["violations"][0]
        self.assertEqual(finding["engine"], "adapter:example")
        self.assertEqual(finding["fullLine"], "clean source")
        self.assertEqual(finding["id"], "example/no-slop")

    def test_malformed_response_retains_bounded_checker_stderr(self) -> None:
        self.adapter("malformed")
        data = self.scan(2)
        self.assertTrue(any("fixture diagnostic: malformed response" in error for error in data["errors"]))

    def test_optional_error_is_visible_and_explicit(self) -> None:
        self.adapter("malformed", required=False)
        data = self.scan(0)
        coverage = next(item for item in data["coverage"] if item["id"] == "example")
        self.assertFalse(coverage["required"])
        self.assertEqual(coverage["status"], "optional-error")
        self.assertTrue(coverage["details"])

    def test_fast_tier_never_runs_commit_adapter(self) -> None:
        record = self.base / "request.json"
        self.adapter("pass", record=str(record))
        self.gate("--file", "src/source.data", expected=0)
        self.assertFalse(record.exists())
        self.scan(0)
        request = json.loads(record.read_text())
        self.assertIsNone(request["files"])
        self.assertEqual(request["scope"], "project")

    def test_config_only_and_deletion_only_commits_still_run_project_checks(self) -> None:
        record = self.base / "request.json"
        self.adapter("pass", record=str(record))
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")
        self.write("package-lock.json", "{}\n")
        self.git("add", "package-lock.json")
        self.gate("--staged", expected=0)
        self.assertTrue(record.exists())
        self.git("commit", "--quiet", "-m", "config-only")
        record.unlink()
        self.git("rm", "src/source.data")
        self.gate("--staged", expected=0)
        self.assertTrue(record.exists())
        self.assertIsNone(json.loads(record.read_text())["files"])

    def test_staged_health_telemetry_does_not_replace_required_failure_policy(self) -> None:
        self.adapter("error")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")
        health = self.root / ".slopgate/cache/checker-health.json"
        for count in (1, 2):
            self.gate("--staged", expected=2)
            state = json.loads(health.read_text())
            self.assertEqual(state["checkers"]["example"]["consecutiveFailures"], count)
        self.adapter("pass")
        self.git("add", ".")
        self.gate("--staged", expected=0)
        state = json.loads(health.read_text())
        self.assertEqual(state["checkers"]["example"]["consecutiveFailures"], 0)
        self.assertIn("lastOk", state["checkers"]["example"])

    def test_partial_staging_untracked_and_removed_worktree_paths_are_rejected(self) -> None:
        self.add_regex()
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")
        self.write("src/source.data", "FORBIDDEN_TOKEN\n")
        self.git("add", "src/source.data")
        self.write("src/source.data", "clean but unstaged\n")
        self.gate("--staged", expected=2)
        self.git("add", "src/source.data")
        self.write("src/untracked.data", "new input\n")
        self.gate("--staged", expected=2)
        self.git("add", "src/untracked.data")
        (self.root / "src/untracked.data").unlink()
        self.gate("--staged", expected=2)

    def test_incomplete_scan_cannot_create_update_or_prune_baseline(self) -> None:
        self.adapter("error")
        baseline = self.root / ".slopgate/baseline.json"
        self.gate("baseline", expected=2)
        self.assertFalse(baseline.exists())
        self.adapter("pass")
        self.gate("baseline", expected=0)
        previous = baseline.read_bytes()
        self.adapter("error")
        for args in (("baseline", "--update"), ("baseline", "--prune"), ("baseline", "--prune", "--update")):
            self.gate(*args, expected=2)
            self.assertEqual(baseline.read_bytes(), previous)

    def test_suppression_does_not_authorize_new_identical_copies(self) -> None:
        self.add_regex()
        self.write("src/source.data", "FORBIDDEN_TOKEN\n")
        entry = {"id": "project/no-slop", "file": "src/source.data", "lineHash": hashlib.sha1(b"FORBIDDEN_TOKEN").hexdigest()}
        self.write(".slopgate/suppressions.json", json.dumps({"version": 1, "entries": [entry]}))
        self.gate("--file", "src/source.data", expected=0)
        self.write("src/source.data", "FORBIDDEN_TOKEN\nFORBIDDEN_TOKEN\n")
        data = self.scan(1, tier="fast")
        self.assertEqual(len(data["violations"]), 1)
        self.assertEqual(data["violations"][0]["line"], 2)

    def test_adapter_cannot_change_and_restage_inputs_while_claiming_success(self) -> None:
        self.adapter("mutate-index")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "stable fixture")
        result = self.gate("--staged", expected=2)
        self.assertIn("tree changed", result.stderr)

    def test_timeout_output_flood_and_exited_parent_are_bounded(self) -> None:
        for behavior in ("sleep", "flood"):
            self.adapter(behavior, timeout_ms=150)
            started = time.monotonic()
            self.scan(2, timeout=5)
            self.assertLess(time.monotonic() - started, 4.0)
        self.adapter("descendant")
        started = time.monotonic()
        self.scan(0, timeout=5)
        self.assertLess(time.monotonic() - started, 4.0)

    def test_exited_adapter_has_no_surviving_descendant(self) -> None:
        marker = self.base / "surviving-child.txt"
        self.adapter("cleanup-proof", survivorMarker=str(marker))
        self.scan(0, timeout=5)
        time.sleep(1.2)
        self.assertFalse(marker.exists(), "adapter returned, but a descendant continued running")

    def test_absolute_editor_path_and_ordinary_relative_path_enforce_same_rule(self) -> None:
        self.add_regex()
        source = self.write("src/editor input.data", "FORBIDDEN_TOKEN\n")
        self.gate("--file", str(source.absolute()), expected=1)
        self.gate("--file", "src/editor input.data", expected=1)
        outside = self.base / "outside.data"
        outside.write_text("FORBIDDEN_TOKEN\n", encoding="utf-8")
        self.gate("--file", str(outside), expected=2)

    def test_partial_gate_configuration_preserves_unspecified_default_tier(self) -> None:
        self.add_regex()
        self.write("src/source.data", "FORBIDDEN_TOKEN\n")
        configuration = (self.root / ".slopgate/config.toml").read_text()
        self.write(".slopgate/config.toml", configuration + '\n[gate]\nstaged=["high"]\n')
        self.gate("--file", "src/source.data", expected=1)
        self.write(".slopgate/config.toml", configuration + '\n[gate]\nfile=["high"]\n')
        self.scan(1)
        self.write(".slopgate/config.toml", configuration + '\n[gate]\nfile=[]\n')
        self.gate("--file", "src/source.data", expected=0)
        self.scan(1)

    def test_shared_engine_id_cannot_be_shadowed_by_external_adapter(self) -> None:
        for identifier in ("regex", "ast"):
            self.config(f'[adapters.{identifier}]\nexecutable={json.dumps(sys.executable)}\n')
            self.gate("scan", "--scope", "repo", expected=2)

    def test_invalid_ux_configuration_does_not_silently_disable_rules(self) -> None:
        for value in ('1', '[]', '"hgh"'):
            self.config(f'[ux]\na11y={value}\n')
            self.gate("scan", "--scope", "repo", expected=2)

    def test_unknown_checker_unknown_config_and_missing_rule_directory_fail(self) -> None:
        for config in ('[checkers.typo-in-checker-name]\n', 'checkerConcurency = 5\n', 'astRules = "./missing"\n', 'checkerConcurrency = 0\n', '[adapters.bad]\nexecutable="x"\ntimeoutMs=0\n'):
            with self.subTest(config=config):
                self.config(config)
                self.gate("scan", "--scope", "repo", expected=2)

    def test_missing_source_and_invalid_regex_are_not_clean(self) -> None:
        self.gate("--file", "src/missing.data", expected=2)
        self.add_regex("(")
        self.gate("--file", "src/source.data", expected=2)

    def test_unicode_paths_and_repository_root_dot(self) -> None:
        self.add_regex()
        self.write("src/שלום data.py", "FORBIDDEN_TOKEN\n")
        self.gate("--file", "src/שלום data.py", expected=1)
        config = (self.root / ".slopgate/config.toml").read_text().replace('["src"]', '["."]')
        self.write(".slopgate/config.toml", config)
        self.gate("--file", "src/שלום data.py", expected=1)

    @unittest.skipIf(os.name == "nt", "Windows filenames cannot contain newline control characters")
    def test_git_newline_paths_use_nul_delimiters(self) -> None:
        self.add_regex()
        self.write("src/new\nline.data", "FORBIDDEN_TOKEN\n")
        self.git("add", ".")
        self.gate("--staged", expected=1)

    def test_parallelism_is_bounded_and_reporting_deterministic(self) -> None:
        state = self.base / "concurrency"
        shutil.copyfile(REPOSITORY / "tests/acceptance/protocol_adapter.py", self.root / "adapter.py")
        config = 'checkerConcurrency = 2\n'
        for i in range(6):
            config += f'[adapters.check{i}]\nexecutable={json.dumps(sys.executable)}\nargs=["adapter.py"]\n[adapters.check{i}.settings]\nbehavior="concurrency"\nstate={json.dumps(str(state))}\n'
        self.config(config)
        # Exercise repeated independent batches so sporadic platform lifecycle
        # or fixture-lock errors cannot hide behind a single successful launch.
        for iteration in range(12):
            with self.subTest(iteration=iteration):
                data = self.scan(0)
                with contextlib.closing(sqlite3.connect(state / "concurrency.sqlite", timeout=5)) as database:
                    self.assertEqual(database.execute("SELECT maximum FROM counters WHERE id = 1").fetchone()[0], 2)
                    self.assertEqual(database.execute("SELECT COUNT(*) FROM active").fetchone()[0], 0)
                ids = [coverage["id"] for coverage in data["coverage"]]
                self.assertEqual(ids, sorted(ids))

    def test_real_ast_rules_apply_outside_typescript_and_valid_fixture_is_clean(self) -> None:
        if shutil.which("ast-grep") is None:
            if REQUIRE_COMPILERS:
                self.fail("required ast-grep test binary was not provisioned")
            self.skipTest("ast-grep not provisioned")
        self.write(".slopgate/rules/no-print.yml", 'id: project-no-print\nlanguage: Python\nrule:\n  pattern: print($$$ARGS)\nseverity: error\nmessage: Use approved logging.\n')
        self.config('astRules="./rules"\n', native_ast=True)
        self.write("src/valid.py", "value = 1\n")
        self.write("src/invalid.py", "print('bad')\n")
        self.gate("--file", "src/valid.py", expected=0)
        self.gate("--file", "src/invalid.py", expected=1)
        data = self.scan(1, tier="fast")
        matching = [finding for finding in data["violations"] if finding["id"] == "project-no-print"]
        self.assertEqual([(finding["file"], finding["line"]) for finding in matching], [("src/invalid.py", 1)])

    def test_linked_worktree_uses_its_own_root(self) -> None:
        self.add_regex()
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "fixture")
        linked = self.base / "linked"
        self.git("worktree", "add", "--quiet", "-b", "acceptance-linked", str(linked))
        self.addCleanup(lambda: execute(["git", "worktree", "remove", "--force", str(linked)], self.root))
        (linked / "src/source.data").write_text("FORBIDDEN_TOKEN\n", encoding="utf-8")
        result = execute([str(BINARY), "--file", "src/source.data", "--config", ".slopgate/config.toml"], linked)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def install_typescript(self) -> None:
        node = shutil.which("node")
        typescript = TOOL_ROOT / "node_modules/typescript/bin/tsc" if TOOL_ROOT else None
        if not node or not typescript or not typescript.exists():
            if REQUIRE_COMPILERS:
                self.fail("required TypeScript fixture compiler was not provisioned")
            self.skipTest("TypeScript fixture compiler not provisioned")
        # Use a project-local real compiler, not a fake diagnostic producer.
        target = self.root / "node_modules/typescript"
        target.parent.mkdir(parents=True)
        shutil.copytree(typescript.parent.parent, target)
        bindir = self.root / "node_modules/.bin"
        bindir.mkdir()
        if os.name == "nt":
            self.write("node_modules/.bin/tsc.cmd", f'@"{node}" "%~dp0\\..\\typescript\\bin\\tsc" %*\r\n')
        else:
            self.write("node_modules/.bin/tsc", f'#!/bin/sh\nexec "{node}" "{target / "bin/tsc"}" "$@"\n').chmod(0o755)

    def test_real_typescript_compiler_checks_entire_configured_project(self) -> None:
        self.install_typescript()
        self.write("tsconfig.json", json.dumps({"compilerOptions": {"strict": True, "noEmit": True}, "include": ["src/**/*.ts"]}))
        self.write("src/untouched.ts", 'export const answer: number = "wrong";\n')
        self.config('[checkers.tsc]\nrequired=true\n', roots=["src"])
        self.gate("--file", "src/source.data", "--tier", "commit", expected=1, timeout=30)
        self.write("src/untouched.ts", 'export const answer: number = 42;\n')
        self.scan(0, timeout=30)

    def test_typescript_solution_references_cannot_silently_pass_without_build_mode(self) -> None:
        self.install_typescript()
        self.write("tsconfig.json", json.dumps({"files": [], "references": [{"path": "./packages/library"}]}))
        self.write("packages/library/tsconfig.json", json.dumps({"compilerOptions": {"composite": True, "strict": True, "outDir": "../../dist/library"}, "include": ["*.ts"]}))
        self.write("packages/library/index.ts", 'export const answer: number = "wrong";\n')
        self.config('[checkers.tsc]\nrequired=true\n')
        error = self.scan(2, timeout=30)
        self.assertTrue(any("build=true" in message for message in error["errors"]))
        self.config('[checkers.tsc]\nrequired=true\nbuild=true\n')
        result = self.scan(1, timeout=30)
        self.assertTrue(any(finding["id"] == "tsc-TS2322" for finding in result["violations"]))
        self.write("packages/library/index.ts", 'export const answer: number = 42;\n')
        self.scan(0, timeout=30)

    def test_real_rust_cargo_adapter_distinguishes_compiler_error_from_infrastructure(self) -> None:
        if not shutil.which("cargo"):
            if REQUIRE_COMPILERS:
                self.fail("required Cargo test compiler was not provisioned")
            self.skipTest("Cargo not provisioned")
        self.write("Cargo.toml", '[package]\nname="slopgate-acceptance-fixture"\nversion="0.0.0"\nedition="2021"\n')
        self.write("src/lib.rs", "pub fn answer() -> u32 { 42 }\n")
        lock = execute(["cargo", "generate-lockfile", "--offline"], self.root)
        self.assertEqual(lock.returncode, 0, lock.stderr)
        self.config('[checkers.cargo-check]\nrequired=true\ntimeout=30\n')
        self.scan(0, timeout=40)
        self.write("src/lib.rs", 'pub fn answer() -> u32 { "wrong" }\n')
        data = self.scan(1, timeout=40)
        self.assertTrue(any(finding["id"] == "cargo-E0308" for finding in data["violations"]))


def main() -> None:
    global BINARY
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    known, remaining = parser.parse_known_args()
    BINARY = known.binary.resolve()
    if not BINARY.is_file():
        parser.error(f"binary does not exist: {BINARY}")
    unittest.main(argv=[sys.argv[0], *remaining], verbosity=2)


if __name__ == "__main__":
    main()
