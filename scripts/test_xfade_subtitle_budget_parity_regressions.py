#!/usr/bin/env python3
"""Unit tests for scripts/generate_xfade_subtitle_budget_parity_regressions.py."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_xfade_subtitle_budget_parity_regressions import generate_all


class GenerateXfadeSubtitleBudgetParityRegressionsTests(unittest.TestCase):
    def test_generates_deterministic_fixtures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = generate_all(Path(tmp))
            self.assertEqual(
                set(paths.keys()),
                {
                    "xfade-primary.y4m",
                    "xfade-secondary.y4m",
                    "two-cues.srt",
                    "burn.srt",
                    "two-cues-subrip.mkv",
                },
            )
            self.assertTrue(paths["xfade-primary.y4m"].read_bytes().startswith(b"YUV4MPEG2 W96 H64"))
            self.assertTrue(paths["xfade-secondary.y4m"].read_bytes().startswith(b"YUV4MPEG2 W96 H64"))
            self.assertIn(b"SubRip cue zero payload with forty bytes", paths["two-cues.srt"].read_bytes())
            self.assertTrue(paths["two-cues-subrip.mkv"].read_bytes().startswith(b"\x1a\x45\xdf\xa3"))


if __name__ == "__main__":
    unittest.main()
