#!/usr/bin/env python3
"""Offline unit tests for scripts/generate_subtitle_spool_regressions.py."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_subtitle_spool_regressions as gen


class SubtitleSpoolRegressionGeneratorTests(unittest.TestCase):
    def test_vtt_sami_and_ass_builders_are_deterministic(self) -> None:
        vtt = gen.build_webvtt_comments_and_styles()
        self.assertTrue(vtt.startswith(b"WEBVTT\n"))
        self.assertIn(b"NOTE multi-line comment", vtt)
        self.assertIn(b"STYLE\n", vtt)
        self.assertIn(b"Visible WebVTT line", vtt)

        sami = gen.build_sami_uppercase_nbsp_clear()
        self.assertIn(b"<SAMI>", sami)
        self.assertIn(b"&NBSP;", sami)
        self.assertIn(b"&Nbsp;", sami)

        ass = gen.build_ass_script_info_and_custom_format()
        self.assertIn(b"[Script Info]", ass)
        self.assertIn(b"[Events]", ass)
        self.assertIn(b"Real ASS cue", ass)

    def test_main_writes_expected_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            old_out = gen.OUT_DIR
            try:
                gen.OUT_DIR = pathlib.Path(tmp)
                gen.main()
                for name in (
                    "webvtt_note_and_style.vtt",
                    "sami_uppercase_nbsp.smi",
                    "ass_script_info_dialogue.ass",
                ):
                    p = gen.OUT_DIR / name
                    self.assertTrue(p.is_file(), name)
                    self.assertGreater(p.stat().st_size, 32, name)
            finally:
                gen.OUT_DIR = old_out


if __name__ == "__main__":
    unittest.main()
