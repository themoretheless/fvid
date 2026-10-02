#!/usr/bin/env python3
"""Dispatch/report policy tests with simulated hardware, not physical CUDA proof."""
import builtins
import json
import pathlib
import sys
import tempfile
import types
import unittest
from unittest.mock import patch
import validate_hw_cuda

REQUIRED = {
    "pipeline::tests": ["resident_cuda_shaders_fuse_geometry_and_keep_one_host_roundtrip",
                        "sampling_shader_matches_independent_crop_and_reflection_reference"],
    "nv12::native_nv12::shader_tests": ["native_nv12_sampling_shader_preserves_pitches_and_matches_cpu",
                                      "native_p010_sampling_shader_preserves_pitches_and_matches_cpu",
                                      "followed_stream_survives_parameter_changes_and_processor_drop"],
    "hw_cuda::layout_tests": ["main10_filter_encodes_hevc_without_host_frame_copies",
                             "vertical_reflection_copies_host_frames_only_when_explicitly_requested"],
}


class ReferencePolicy(unittest.TestCase):
    def hardware(self, command, **kwargs):
        self.calls.append(command)
        if command[0] == "nvidia-smi":
            output = "simulated hardware\n"
        elif command[0] == "cargo":
            selector = command[command.index("--lib") + 1]
            names = REQUIRED[selector]
            output = "".join(f"test suite::{name} ... ok\n" for name in names)
            output += f"test result: ok. {len(names)} passed; 0 failed; 0 ignored;\n"
        else:
            raise AssertionError(f"unexpected ordinary subprocess: {command}")
        return types.SimpleNamespace(returncode=0, stdout=output, stderr="")

    def setUp(self):
        self.calls = []

    def test_default_runs_required_hardware_suites_without_reference_import(self):
        original = builtins.__import__

        def guarded_import(name, *args, **kwargs):
            if name == "benchmark_hw_cuda_reference":
                raise AssertionError("ordinary validation imported FFmpeg reference")
            return original(name, *args, **kwargs)

        with tempfile.TemporaryDirectory() as folder:
            destination = pathlib.Path(folder) / "report.json"
            with patch.object(validate_hw_cuda.subprocess, "run", side_effect=self.hardware), \
                 patch.object(builtins, "__import__", side_effect=guarded_import):
                report = validate_hw_cuda.main(["--binary", "missing", "--report", str(destination)])
            self.assertEqual(json.loads(destination.read_text()), report)
            self.assertEqual(report["status"], "passed")
            self.assertFalse(report["reference_requested"])
            self.assertFalse(report["reference_completed"])
            self.assertFalse(report["provided_cli_checks_completed"])
            self.assertEqual([r["passed"] for r in report["native_tests"]], [2, 3, 2])
            self.assertEqual(len(self.calls), 4)
            self.assertTrue(all("--ignored" in c for c in self.calls[1:]))
            self.assertIn("cuda-hw", self.calls[-1])

    def test_explicit_reference_dispatches_after_physical_qualification(self):
        reference_calls = []

        def reference(run, binary, report, checks):
            self.assertEqual(len(self.calls), 4)
            reference_calls.append(binary)
            checks.append("simulated reference checks")

        module = types.SimpleNamespace(run_reference=reference)
        with tempfile.TemporaryDirectory() as folder, \
             patch.object(validate_hw_cuda.subprocess, "run", side_effect=self.hardware), \
             patch.dict(sys.modules, {"benchmark_hw_cuda_reference": module}):
            report = validate_hw_cuda.main(["--binary", "missing", "--benchmark-reference",
                                           "--report", str(pathlib.Path(folder) / "report.json")])
        self.assertEqual(len(reference_calls), 1)
        self.assertTrue(report["reference_requested"])
        self.assertTrue(report["reference_completed"])
        self.assertTrue(report["provided_cli_checks_completed"])

    def test_zero_executed_tests_save_failure_and_never_reach_reference(self):
        def empty(command, **kwargs):
            return types.SimpleNamespace(returncode=0, stdout="test result: ok. 0 passed; 0 failed; 7 ignored;", stderr="")

        with tempfile.TemporaryDirectory() as folder:
            destination = pathlib.Path(folder) / "report.json"
            with patch.object(validate_hw_cuda.subprocess, "run", side_effect=empty), \
                 patch.dict(sys.modules, {"benchmark_hw_cuda_reference": None}):
                with self.assertRaisesRegex(RuntimeError, "requires at least 2"):
                    validate_hw_cuda.main(["--benchmark-reference", "--report", str(destination)])
            report = json.loads(destination.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertFalse(report["reference_completed"])
            self.assertFalse(report["provided_cli_checks_completed"])

    def test_reference_failure_is_published_as_failure(self):
        def failure(*args):
            raise RuntimeError("simulated reference mismatch")

        with tempfile.TemporaryDirectory() as folder:
            destination = pathlib.Path(folder) / "report.json"
            with patch.object(validate_hw_cuda.subprocess, "run", side_effect=self.hardware), \
                 patch.dict(sys.modules, {"benchmark_hw_cuda_reference": types.SimpleNamespace(run_reference=failure)}):
                with self.assertRaisesRegex(RuntimeError, "reference mismatch"):
                    validate_hw_cuda.main(["--benchmark-reference", "--report", str(destination)])
            report = json.loads(destination.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertFalse(report["reference_completed"])
            self.assertEqual(report["error"], "simulated reference mismatch")


if __name__ == "__main__":
    unittest.main()
