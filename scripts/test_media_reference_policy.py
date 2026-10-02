#!/usr/bin/env python3
"""Mocked validation dispatch and report guards; not executed codec evidence."""
import builtins
import json
import pathlib
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import validate_media
from media_qualification import require_cli_reference_report


class MediaPolicy(unittest.TestCase):
    def test_native_runs_policy_and_both_test_suites_without_reference_import(self):
        calls = []
        original = builtins.__import__

        def guarded(name, *args, **kwargs):
            if name == "benchmark_media_reference":
                raise AssertionError("ordinary validation imported external reference")
            return original(name, *args, **kwargs)

        def subprocess(command, **kwargs):
            calls.append(command)
            self.assertNotIn(command[0], ["ffmpeg", "ffprobe"])
            output = "native policy passed" if command[0] == "python3" else (
                "test result: ok. 3 passed; 0 failed; 2 ignored;\n"
                "test result: ok. 7 passed; 0 failed; 0 ignored;\n")
            return types.SimpleNamespace(returncode=0, stdout=output, stderr="")

        with tempfile.TemporaryDirectory() as folder, \
             patch.object(validate_media.subprocess, "run", side_effect=subprocess), \
             patch.object(builtins, "__import__", side_effect=guarded):
            destination = pathlib.Path(folder) / "native.json"
            report = validate_media.main(["--report", str(destination)])
            self.assertEqual(json.loads(destination.read_text()), report)
        self.assertEqual(len(calls), 3)
        self.assertEqual(calls[0][0], "python3")
        self.assertTrue(all("--no-default-features" in command for command in calls[1:]))
        self.assertIn("--tests", calls[-1])
        self.assertEqual([c["passed"] for c in report["checks"][1:]], [10, 10])
        self.assertFalse(report["reference_completed"])
        self.assertFalse(report["provided_cli_checks_completed"])
        self.assertEqual(report["status"], "passed")

    def test_empty_executed_suite_saves_failed_report(self):
        output = types.SimpleNamespace(returncode=0, stdout="test result: ok. 0 passed; 0 failed; 5 ignored;", stderr="")
        with tempfile.TemporaryDirectory() as folder, \
             patch.object(validate_media.subprocess, "run", return_value=output):
            destination = pathlib.Path(folder) / "native.json"
            with self.assertRaisesRegex(RuntimeError, "no successful executed tests"):
                validate_media.main(["--report", str(destination)])
            self.assertEqual(json.loads(destination.read_text())["status"], "failed")

    def test_explicit_reference_preserves_complete_returned_corpus(self):
        calls = []
        corpus = {"status": "passed", "checks": [{"name": "reference", "status": "passed"}] * 524,
                  "binary_sha256": "synthetic", "source_sha256": {"reference.py": "synthetic"}}

        def reference(argv):
            calls.append(argv)
            return corpus

        with tempfile.TemporaryDirectory() as folder, \
             patch.object(validate_media, "native_validation", side_effect=AssertionError("reference ran ordinary suite")), \
             patch.dict(sys.modules, {"benchmark_media_reference": types.SimpleNamespace(main=reference)}):
            destination = pathlib.Path(folder) / "reference.json"
            report = validate_media.main(["--binary", "synthetic", "--benchmark-reference", "--report", str(destination)])
            self.assertEqual(calls, [["--binary", "synthetic", "--report", str(destination.resolve())]])
            self.assertEqual(report["checks"], corpus["checks"])
            self.assertEqual(report["source_sha256"], corpus["source_sha256"])
            require_cli_reference_report(json.loads(destination.read_text()), "synthetic")

    def test_reference_failure_replaces_stale_success(self):
        def failure(argv):
            raise RuntimeError("reference mismatch")

        with tempfile.TemporaryDirectory() as folder, \
             patch.dict(sys.modules, {"benchmark_media_reference": types.SimpleNamespace(main=failure)}):
            destination = pathlib.Path(folder) / "reference.json"
            destination.write_text('{"status":"passed"}')
            with self.assertRaisesRegex(RuntimeError, "reference mismatch"):
                validate_media.main(["--benchmark-reference", "--report", str(destination)])
            report = json.loads(destination.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertFalse(report["reference_completed"])

    def test_gate_requires_complete_reference_scope_and_matching_binary(self):
        historical = {"status": "passed", "checks": [{"status": "passed"}] * 524, "binary_sha256": "synthetic"}
        require_cli_reference_report(historical, "synthetic")
        for change in [{"checks": [{"status": "passed"}] * 523}, {"status": "failed"},
                       {"checks": [{"status": "failed"}] * 524}, {"binary_sha256": "other"},
                       {"reference_requested": False}, {"reference_completed": True}]:
            with self.assertRaises(RuntimeError):
                require_cli_reference_report(dict(historical, **change), "synthetic")
        current = dict(historical, reference_requested=True, reference_completed=True, provided_cli_checks_completed=True)
        require_cli_reference_report(current, "synthetic")


if __name__ == "__main__":
    unittest.main()
