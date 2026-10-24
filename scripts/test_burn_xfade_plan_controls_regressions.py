#!/usr/bin/env python3
"""Offline unit test for Batch 218 (`FVID-0225`) synthetic Y4M and SRT fixtures."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_burn_xfade_plan_controls_regressions as gen


class BurnXfadePlanControlsFixtureTest(unittest.TestCase):
    def test_generated_y4m_and_srt_fixtures_are_deterministic_and_match_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = pathlib.Path(tmp)
            generated = gen.generate_all(tmp_dir)
            self.assertEqual(
                set(generated.keys()),
                {
                    "burn-main-96x64.y4m",
                    "burn-cues.srt",
                    "xfade-main-4x4.y4m",
                    "xfade-other-4x4.y4m",
                },
            )
            for name, payload in generated.items():
                repo_file = gen.FIXTURE_DIR / name
                self.assertTrue(repo_file.is_file(), f"missing fixture {repo_file}")
                self.assertEqual(repo_file.read_bytes(), payload, f"mismatch for {name}")
                if name.endswith(".y4m"):
                    self.assertTrue(payload.startswith(b"YUV4MPEG2 "))
                    self.assertEqual(payload.count(b"FRAME\n"), 3)
                else:
                    self.assertIn(b"-->", payload)


if __name__ == "__main__":
    unittest.main()
