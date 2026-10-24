#!/usr/bin/env python3
"""Offline unit tests for Y4M concat/trim/export and burn/xfade control regression fixtures."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_y4m_burn_xfade_controls_regressions as gen


class Y4mBurnXfadeControlsFixtureTests(unittest.TestCase):
    def test_generated_fixtures_match_committed_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = gen.generate(pathlib.Path(tmp))
            for name, temp_path in generated.items():
                committed = gen.FIXTURES / temp_path.name
                self.assertTrue(committed.is_file(), f"missing committed fixture {committed}")
                self.assertEqual(temp_path.read_bytes(), committed.read_bytes(), name)

        primary = (gen.FIXTURES / "y4m-burn-xfade-primary.y4m").read_bytes()
        secondary = (gen.FIXTURES / "y4m-burn-xfade-secondary.y4m").read_bytes()
        subs = (gen.FIXTURES / "y4m-burn-xfade.srt").read_text("utf-8")

        self.assertTrue(primary.startswith(b"YUV4MPEG2 W32 H16 F25:1 Ip A1:1 C420\n"))
        self.assertEqual(primary.count(b"FRAME\n"), 3)
        self.assertTrue(secondary.startswith(b"YUV4MPEG2 W32 H16 F25:1 Ip A1:1 C420\n"))
        self.assertEqual(secondary.count(b"FRAME\n"), 3)
        self.assertIn("-->", subs)


if __name__ == "__main__":
    unittest.main()
