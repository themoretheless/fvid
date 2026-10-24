#!/usr/bin/env python3
"""Offline unit test for synthetic video export budget/control regression fixtures."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_video_export_controls_regressions as gen


class VideoExportControlsFixturesTest(unittest.TestCase):
    def test_generated_fixtures_match_committed_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = gen.generate(pathlib.Path(tmp))
            for name, temp_path in generated.items():
                committed_path = gen.FIXTURES / temp_path.name
                self.assertTrue(committed_path.is_file(), f"missing {committed_path}")
                self.assertEqual(
                    temp_path.read_bytes(),
                    committed_path.read_bytes(),
                    f"fixture {name} diverged from generator",
                )
                self.assertLessEqual(temp_path.stat().st_size, 512)


if __name__ == "__main__":
    unittest.main()
