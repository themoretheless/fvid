#!/usr/bin/env python3
"""Unit tests for the FVID-0259 lossless export and Y4M trim/concat budget parity fixture generator."""

from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_lossless_y4m_budget_parity_regressions as gen


class LosslessY4mBudgetParityFixtureTests(unittest.TestCase):
    def test_generator_is_deterministic_and_matches_checked_in_fixtures(self) -> None:
        expected = gen.generate()
        self.assertEqual(
            sorted(expected.keys()),
            [
                "lossless-textured.y4m",
                "manifest.json",
                "single-video-avc.mp4",
                "single-video-ffv1.mkv",
            ],
        )
        for name, data in expected.items():
            path = gen.OUT / name
            self.assertTrue(path.exists(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture drift in {name}")

    def test_fixture_signatures_and_sizes(self) -> None:
        expected = gen.generate()
        self.assertTrue(expected["lossless-textured.y4m"].startswith(b"YUV4MPEG2 W16 H16 F2:1"))
        self.assertTrue(expected["single-video-ffv1.mkv"].startswith(b"\x1a\x45\xdf\xa3"))
        self.assertEqual(expected["single-video-avc.mp4"][4:8], b"ftyp")


if __name__ == "__main__":
    unittest.main()
