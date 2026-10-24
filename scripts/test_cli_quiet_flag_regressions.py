#!/usr/bin/env python3
"""Offline unit test verifying CLI quiet/memory/progress regression fixtures."""

from __future__ import annotations

import pathlib
import unittest

import generate_cli_quiet_flag_regressions as gen


class TestCliQuietFlagFixtures(unittest.TestCase):
    def test_fixtures_match_generator(self) -> None:
        wav_path = gen.OUT_DIR / "tone.wav"
        y4m_path = gen.OUT_DIR / "tiny.y4m"
        srt_path = gen.OUT_DIR / "tiny.srt"
        self.assertTrue(wav_path.is_file(), f"missing {wav_path}")
        self.assertTrue(y4m_path.is_file(), f"missing {y4m_path}")
        self.assertTrue(srt_path.is_file(), f"missing {srt_path}")
        self.assertEqual(wav_path.read_bytes(), gen.build_tone_wav())
        self.assertEqual(y4m_path.read_bytes(), gen.build_tiny_y4m())
        self.assertEqual(srt_path.read_bytes(), gen.build_tiny_srt())


if __name__ == "__main__":
    unittest.main()
