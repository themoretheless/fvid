#!/usr/bin/env python3
"""Mocked K-Lite dispatch/report policy; these tests do not prove codec coverage."""
import builtins
import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import validate_klite_coverage as validator
import validate_media
import fetch_klite_samples


class KLitePolicy(unittest.TestCase):
    def test_sample_preparation_requires_opt_in_before_import_or_subprocess(self):
        original = builtins.__import__
        def guarded(name, *args, **kwargs):
            if name == "benchmark_fetch_klite_samples":
                raise AssertionError("sample preparation imported without reference opt-in")
            return original(name, *args, **kwargs)
        with patch("builtins.__import__", side_effect=guarded), \
                patch.object(validate_media.subprocess, "run", side_effect=AssertionError("unexpected subprocess")), \
                contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as error:
                fetch_klite_samples.main(["--auto"])
            self.assertEqual(error.exception.code, 2)
            spec = importlib.util.spec_from_file_location("sample_import_check", fetch_klite_samples.__file__)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as help_exit:
                module.main(["--help"])
            self.assertEqual(help_exit.exception.code, 0)

    def test_sample_preparation_preserves_options_and_failure_exit_code(self):
        calls = []
        def reference(argv):
            calls.append(argv)
            return 1
        with patch.dict(sys.modules, {"benchmark_fetch_klite_samples": types.SimpleNamespace(main=reference)}):
            code = fetch_klite_samples.main(["--benchmark-reference", "--auto", "--force", "--verbose",
                                             "--add", "row-a", "local-synthetic", "--add", "row-b", "https://example.invalid/synthetic"])
        self.assertEqual(code, 1)
        self.assertEqual(calls, [["--force", "--verbose", "--auto", "--add", "row-a", "local-synthetic",
                                  "--add", "row-b", "https://example.invalid/synthetic"]])

    def test_native_default_keeps_historical_reference_report(self):
        with tempfile.TemporaryDirectory() as folder, patch.object(validator, "ROOT", pathlib.Path(folder)), \
                patch.object(validator, "native_validation", return_value=None):
            benchmark = pathlib.Path(folder) / "benchmarks"
            benchmark.mkdir()
            historical = benchmark / "klite-coverage.json"
            historical.write_text("historical full reference evidence")
            report = validator.main([])
            self.assertEqual(historical.read_text(), "historical full reference evidence")
            self.assertEqual(json.loads((benchmark / "native-klite-validation.json").read_text()), report)

    def test_import_and_help_do_not_inventory_external_tools(self):
        with patch.object(validate_media.subprocess, "run", side_effect=AssertionError("unexpected subprocess")):
            spec = importlib.util.spec_from_file_location("klite_import_check", validator.__file__)
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as error:
                module.main(["--help"])
            self.assertEqual(error.exception.code, 0)

    def test_native_runs_all_suites_without_reference_import_or_provided_binary(self):
        calls = []
        original = builtins.__import__
        def guarded(name, *args, **kwargs):
            if name == "benchmark_klite_reference":
                raise AssertionError("native validation imported external oracle")
            return original(name, *args, **kwargs)
        def execute(command, **kwargs):
            calls.append(command)
            self.assertNotIn(command[0], ["ffmpeg", "ffprobe", "absent-cli"])
            output = "native dependency policy passed" if command[0] == "python3" else (
                "test result: ok. 10 passed; 0 failed; 1 ignored;")
            return types.SimpleNamespace(returncode=0, stdout=output, stderr="")
        with tempfile.TemporaryDirectory() as folder, patch("builtins.__import__", side_effect=guarded), \
                patch.object(validate_media.subprocess, "run", side_effect=execute):
            destination = pathlib.Path(folder) / "native.json"
            report = validator.main(["--binary", "absent-cli", "--out", str(destination)])
            self.assertEqual(json.loads(destination.read_text()), report)
        self.assertEqual(len(calls), 3)
        self.assertTrue(all("--no-default-features" in command for command in calls[1:]))
        self.assertIn("--lib", calls[1])
        self.assertIn("--tests", calls[2])
        self.assertEqual(report["status"], "passed")
        self.assertFalse(report["reference_requested"])
        self.assertFalse(report["reference_completed"])
        self.assertFalse(report["provided_cli_checks_completed"])
        self.assertNotIn("rows", report)

    def test_reference_preserves_rows_and_forwards_every_probe_option(self):
        calls = []
        corpus = {"total": 2, "rows": [{"codec": "h264"}, {"codec": "aac"}],
                  "native_decode": [], "inventory": {"synthetic": True},
                  "reference_requested": False, "scope": "untrusted old scope"}
        def reference(argv):
            calls.append(argv)
            pathlib.Path(argv[argv.index("--out") + 1]).write_text(json.dumps(corpus))
            return 0
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(validator, "native_validation", side_effect=AssertionError("wrong mode")), \
                patch.dict(sys.modules, {"benchmark_klite_reference": types.SimpleNamespace(main=reference)}):
            destination = pathlib.Path(folder) / "reference.json"
            report = validator.main(["--benchmark-reference", "--binary", "synthetic-cli", "--out", str(destination),
                                     "--markdown", "matrix.md", "--native-probe", "audio", "--native-video-probe", "video", "--mcp-binary", "mcp"])
            self.assertEqual(report["rows"], corpus["rows"])
            self.assertEqual(report["inventory"], corpus["inventory"])
            self.assertEqual(json.loads(destination.read_text()), report)
            argv = calls[0]
            self.assertNotEqual(argv[argv.index("--out") + 1], str(destination))
        self.assertEqual(len(calls), 1)
        self.assertEqual(argv[:2], ["--binary", "synthetic-cli"])
        self.assertEqual(argv[4:], ["--markdown", "matrix.md", "--native-probe", "audio", "--native-video-probe", "video", "--mcp-binary", "mcp"])
        self.assertEqual(report["scope"], "full K-Lite external reference corpus")
        self.assertTrue(report["reference_requested"])
        self.assertTrue(report["reference_completed"])
        self.assertTrue(report["provided_cli_checks_completed"])

    def test_successful_reference_report_remains_reproducible(self):
        def reference(argv):
            pathlib.Path(argv[argv.index("--out") + 1]).write_text(json.dumps({"total": 1, "rows": [{"codec": "synthetic"}]}))
            return 0
        with tempfile.TemporaryDirectory() as folder, \
                patch.dict(sys.modules, {"benchmark_klite_reference": types.SimpleNamespace(main=reference)}):
            destination = pathlib.Path(folder) / "reference.json"
            argv = ["--benchmark-reference", "--out", str(destination)]
            validator.main(argv)
            first = destination.read_bytes()
            validator.main(argv)
            self.assertEqual(destination.read_bytes(), first)

    def test_failed_reference_retains_current_rows_and_replaces_stale_success(self):
        def reference(argv):
            pathlib.Path(argv[argv.index("--out") + 1]).write_text(json.dumps({"total": 1, "rows": [{"codec": "current-failure"}]}))
            return 1
        with tempfile.TemporaryDirectory() as folder, \
                patch.dict(sys.modules, {"benchmark_klite_reference": types.SimpleNamespace(main=reference)}):
            destination = pathlib.Path(folder) / "reference.json"
            destination.write_text('{"status":"passed","rows":[{"codec":"stale-success"}]}')
            with self.assertRaisesRegex(RuntimeError, "exit code 1"):
                validator.main(["--benchmark-reference", "--out", str(destination)])
            report = json.loads(destination.read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["rows"], [{"codec": "current-failure"}])
        self.assertFalse(report["reference_completed"])
        self.assertFalse(report["provided_cli_checks_completed"])

    def test_success_without_current_complete_rows_is_refused(self):
        for corpus in [None, {"total": 0, "rows": []}, {"total": 2, "rows": [{"codec": "h264"}]}]:
            def reference(argv):
                if corpus is not None:
                    pathlib.Path(argv[argv.index("--out") + 1]).write_text(json.dumps(corpus))
                return 0
            with self.subTest(corpus=corpus), tempfile.TemporaryDirectory() as folder, \
                    patch.dict(sys.modules, {"benchmark_klite_reference": types.SimpleNamespace(main=reference)}):
                destination = pathlib.Path(folder) / "reference.json"
                destination.write_text('{"status":"passed","total":1,"rows":[{"codec":"stale"}]}')
                with self.assertRaisesRegex(RuntimeError, "no complete row report"):
                    validator.main(["--benchmark-reference", "--out", str(destination)])
                report = json.loads(destination.read_text())
                self.assertEqual(report["status"], "failed")
                self.assertFalse(report["reference_completed"])

    def test_reference_import_failure_is_published_as_failure(self):
        original = builtins.__import__
        def missing(name, *args, **kwargs):
            if name == "benchmark_klite_reference":
                raise FileNotFoundError("external oracle unavailable")
            return original(name, *args, **kwargs)
        with tempfile.TemporaryDirectory() as folder, patch("builtins.__import__", side_effect=missing):
            destination = pathlib.Path(folder) / "reference.json"
            with self.assertRaisesRegex(FileNotFoundError, "oracle unavailable"):
                validator.main(["--benchmark-reference", "--out", str(destination)])
            report = json.loads(destination.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertFalse(report["reference_completed"])

    def test_native_mode_cannot_rewrite_reference_markdown(self):
        with tempfile.TemporaryDirectory() as folder, \
                patch.object(validator, "native_validation", side_effect=AssertionError("invalid options ran tests")), \
                contextlib.redirect_stderr(io.StringIO()):
            document = pathlib.Path(folder) / "matrix.md"
            document.write_text("original reference matrix")
            with self.assertRaises(SystemExit) as error:
                validator.main(["--markdown", str(document)])
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(document.read_text(), "original reference matrix")


if __name__ == "__main__":
    unittest.main()
