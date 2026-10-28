#!/usr/bin/env python3
"""Unit tests for scripts/generate_pcm_loudness_qualification_regressions.py."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_pcm_loudness_qualification_regressions as gen


class GeneratePcmLoudnessQualificationRegressionsTests(unittest.TestCase):
    def test_generator_is_deterministic_and_matches_committed_fixtures(self) -> None:
        generated = gen.generate()
        manifest = json.loads(generated["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0267")
        for name, expected_sha in manifest["sha256"].items():
            self.assertEqual(hashlib.sha256(generated[name]).hexdigest(), expected_sha)
            committed = (gen.OUT / name).read_bytes()
            self.assertEqual(committed, generated[name], f"mismatch for {name}")

    def test_write_fixtures_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            files = gen.write_fixtures(Path(tmp))
            self.assertIn("discrete-3ch-48k.flac", files)
            self.assertIn("mono-20k.flac", files)
            self.assertIn("video-4096-audio-1920.mkv", files)


if __name__ == "__main__":
    unittest.main()
