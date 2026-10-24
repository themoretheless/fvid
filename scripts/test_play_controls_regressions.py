#!/usr/bin/env python3
"""Offline unit tests for scripts/generate_play_controls_regressions.py (FVID-0202)."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.generate_play_controls_regressions import (
    OUT_DIR,
    build_ass_with_arrow_in_dialogue,
    build_two_frame_y4m,
    generate,
)


class TestPlayControlsRegressions(unittest.TestCase):
    def test_two_frame_y4m_structure(self) -> None:
        y4m = build_two_frame_y4m()
        self.assertTrue(y4m.startswith(b"YUV4MPEG2 W2 H2 F10:1 Ip C420jpeg\n"))
        self.assertEqual(y4m.count(b"FRAME\n"), 2)

    def test_ass_fixture_has_arrow_and_events_section(self) -> None:
        ass = build_ass_with_arrow_in_dialogue()
        self.assertIn("[Script Info]", ass)
        self.assertIn("[Events]", ass)
        self.assertIn("Step A --> Step B", ass)
        self.assertIn("Fake header dialogue", ass)
        self.assertIn("Trailing non-event dialogue", ass)

    def test_generate_matches_checked_in_fixtures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            manifest = generate(tmp_dir)
            self.assertEqual(manifest["improvement_id"], "FVID-0202")
            for item in manifest["fixtures"]:
                name = item["file"]
                self.assertEqual(
                    (tmp_dir / name).read_bytes(),
                    (OUT_DIR / name).read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
