#!/usr/bin/env python3
"""Unit tests for scripts/generate_mix_merge_prefix_budget_wave_info_regressions.py."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_mix_merge_prefix_budget_wave_info_regressions as gen


class GenerateMixMergePrefixBudgetWaveInfoRegressionsTests(unittest.TestCase):
    def test_generator_is_deterministic_and_matches_committed_fixtures(self) -> None:
        generated = gen.generate()
        manifest = json.loads(generated["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0268")
        for name, expected_sha in manifest["sha256"].items():
            self.assertEqual(hashlib.sha256(generated[name]).hexdigest(), expected_sha)
            committed = (gen.OUT / name).read_bytes()
            self.assertEqual(committed, generated[name], f"mismatch for {name}")

    def test_write_fixtures_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            files = gen.write_fixtures(Path(tmp))
            self.assertIn("prefix-a.flac", files)
            self.assertIn("prefix-b.flac", files)
            self.assertIn("fourcc-tags.opus", files)
            self.assertIn("fourcc-tags.mp3", files)


if __name__ == "__main__":
    unittest.main()
