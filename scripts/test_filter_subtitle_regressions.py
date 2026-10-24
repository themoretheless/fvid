#!/usr/bin/env python3
"""Offline unit tests for scripts/generate_filter_subtitle_regressions.py (Batch 196 / FVID-0203)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from generate_filter_subtitle_regressions import (
    build_spaced_format_ass,
    build_uppercase_entity_and_coords_srt,
    generate_all,
)


class FilterSubtitleRegressionsGeneratorTests(unittest.TestCase):
    def test_srt_contains_dot_timestamps_coords_and_uppercase_entities(self) -> None:
        raw = build_uppercase_entity_and_coords_srt().decode("utf-8")
        self.assertIn("00:00:01.000 --> 00:00:02.500 X1:100 X2:200", raw)
        self.assertIn("R&AMP;D &NBSP; &LT;ok&GT;", raw)
        self.assertIn("&QUOT;hello&QUOT; &APOS;world&APOS;", raw)

    def test_ass_contains_spaced_keys_and_variable_fraction_digits(self) -> None:
        raw = build_spaced_format_ass().decode("utf-8")
        self.assertIn("[Events]\nformat : Layer", raw)
        self.assertIn("dialogue : 0,0:00:01.250,0:00:02.5,Default", raw)

    def test_generate_all_writes_expected_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            written = generate_all(Path(tmp))
            self.assertEqual(
                set(written.keys()),
                {"uppercase-entity-coords.srt", "spaced-format-millis.ass"},
            )
            for path in written.values():
                self.assertTrue(path.is_file())
                self.assertGreater(path.stat().st_size, 0)


if __name__ == "__main__":
    unittest.main()
