#!/usr/bin/env python3
"""Unit checks for the comparison oracle, corpus identity and summary math."""
import json
import tempfile
import unittest
from pathlib import Path

import compare_performance as bench


class ComparisonTests(unittest.TestCase):
    def report(self):
        return {"coverage": [{"id": "regex", "status": "complete", "elapsedMs": 3}],
                "violations": [{"id": "probe", "line": 1, "text": "evidence"}], "errors": []}

    def test_only_coverage_elapsed_is_ignored(self):
        before = self.report()
        after = self.report()
        after["coverage"][0]["elapsedMs"] = 400
        self.assertEqual(bench.semantic_report(json.dumps(before)), bench.semantic_report(json.dumps(after)))
        after["coverage"][0]["status"] = "disabled"
        self.assertNotEqual(bench.semantic_report(json.dumps(before)), bench.semantic_report(json.dumps(after)))

    def test_findings_errors_order_and_nested_fields_are_not_ignored(self):
        before = self.report()
        for changed in [
            {"violations": []}, {"errors": ["missing tool"]},
            {"violations": [{"id": "probe", "line": 2, "text": "evidence"}]},
            {"elapsedMs": 44}, {"coverage": []},
        ]:
            after = dict(before, **changed)
            self.assertNotEqual(bench.semantic_report(json.dumps(before)), bench.semantic_report(json.dumps(after)))

    def test_only_ast_scratch_nonce_is_normalized(self):
        prefix = "sg: summary|project: isProject=true,projectDir="
        first = prefix + str(Path(tempfile.gettempdir()) / "slopgate-sg-abc123")
        second = prefix + str(Path(tempfile.gettempdir()) / "slopgate-sg-def456")
        def report(detail, stage="ast"):
            return json.dumps({"coverage": [{"id": stage, "details": [detail]}], "violations": []})
        self.assertEqual(bench.semantic_report(report(first)), bench.semantic_report(report(second)))
        self.assertNotEqual(bench.semantic_report(report(first, "regex")), bench.semantic_report(report(second, "regex")))
        self.assertNotEqual(bench.semantic_report(report(first)), bench.semantic_report(report(second.replace("isProject=true", "isProject=false"))))
        self.assertNotEqual(bench.semantic_report(report(first)), bench.semantic_report(report(second + ",extra=changed")))

    def test_malformed_or_unstructured_output_is_rejected(self):
        for text in ["", "null", "[]", "{}", '{"coverage":null}']:
            with self.assertRaises((ValueError, RuntimeError)):
                bench.semantic_report(text)

    def test_nearest_rank_percentile_and_raw_samples(self):
        samples = [float(n) for n in range(20, 0, -1)]
        result = bench.summary(samples)
        self.assertEqual(result["medianMs"], 10.5)
        self.assertEqual(result["p95Ms"], 19.0)
        self.assertEqual(result["rawMs"], samples)

    def test_pairing_interval_is_repeatable(self):
        before = [2.0, 4.0, 6.0]
        after = [1.0, 2.0, 3.0]
        self.assertEqual(bench.paired_interval(before, after), [2.0, 2.0])

    def test_corpus_identity_tracks_bytes_and_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bench.write(root, "src/file.ts", "first\n")
            first = bench.corpus_identity(root)
            self.assertEqual(first["files"], 1)
            self.assertEqual(first["sourceBytes"], 6)
            bench.write(root, "src/file.ts", "other\n")
            second = bench.corpus_identity(root)
            self.assertNotEqual(first["sha256"], second["sha256"])
            (root / "src/file.ts").rename(root / "src/renamed.ts")
            self.assertNotEqual(second["sha256"], bench.corpus_identity(root)["sha256"])


if __name__ == "__main__":
    unittest.main()
