#!/usr/bin/env python3
"""Unit tests for scripts/generate_matroska_prefix_budget_mp4_wave_info_regressions.py."""

from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_matroska_prefix_budget_mp4_wave_info_regressions as gen


class GenerateMatroskaPrefixBudgetMp4WaveInfoRegressionsTest(unittest.TestCase):
    def test_deterministic_generation_matches_committed_fixtures(self) -> None:
        generated = gen.generate()
        self.assertEqual(
            sorted(generated.keys()),
            [
                "manifest.json",
                "prefix-flac.mka",
                "prefix-pcm.mka",
                "tagged-encoder.m4a",
            ],
        )
        for name, expected in generated.items():
            committed = (gen.OUT / name).read_bytes()
            self.assertEqual(committed, expected, f"fixture mismatch for {name}")

    def test_headers_and_tags_present(self) -> None:
        generated = gen.generate()
        self.assertTrue(generated["prefix-flac.mka"].startswith(b"\x1a\x45\xdf\xa3"))
        self.assertTrue(generated["prefix-pcm.mka"].startswith(b"\x1a\x45\xdf\xa3"))
        self.assertIn(b"fvid-mp4-muxer 1.0", generated["tagged-encoder.m4a"])


if __name__ == "__main__":
    unittest.main()
