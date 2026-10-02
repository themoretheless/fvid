#!/usr/bin/env python3
"""Explicit generator verification; not invoked by ordinary Cargo tests."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("fixture_generator", HERE / "generate_playback_error_samples.py")
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class SeededGeneratorTests(unittest.TestCase):
    def test_exact_existing_corpus_with_no_external_process(self):
        base = generator.ROOT / "tests/fixtures/playback-errors"
        expected = json.loads((base / "synthetic-playback-generated.json").read_text())
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "fixtures"
            with patch.object(generator, "OUT", output), patch("subprocess.Popen", side_effect=AssertionError("external process forbidden")):
                generator.main()
            actual = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in output.iterdir()}
            self.assertEqual(actual, expected)

    def test_corrupt_manifest_refused_before_output_creation(self):
        original = json.loads
        def corrupt(text):
            result = original(text)
            result["control.mp4"] = "0" * 64
            return result
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "fixtures"
            with patch.object(generator, "OUT", output), patch.object(generator.json, "loads", side_effect=corrupt):
                with self.assertRaisesRegex(ValueError, "integrity failure: control.mp4"):
                    generator.main()
            self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
