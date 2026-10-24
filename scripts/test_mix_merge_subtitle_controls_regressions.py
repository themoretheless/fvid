#!/usr/bin/env python3
"""Offline unit test for Batch 211 (FVID-0218) synthetic fixtures."""

from pathlib import Path
import tempfile
import unittest

import generate_mix_merge_subtitle_controls_regressions as gen


class TestMixMergeSubtitleControlsFixtures(unittest.TestCase):
    def test_fixtures_are_deterministic_and_valid(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = gen.generate(Path(tmp))
            for key, path in generated.items():
                committed = gen.FIXTURES / path.name
                self.assertTrue(committed.is_file(), f"missing committed fixture {committed}")
                self.assertEqual(path.read_bytes(), committed.read_bytes(), f"mismatch for {key}")
            self.assertTrue(generated["wav_a"].read_bytes().startswith(b"RIFF"))
            self.assertTrue(generated["video"].read_bytes().startswith(b"YUV4MPEG2 "))
            self.assertIn(b"-->", generated["srt"].read_bytes())


if __name__ == "__main__":
    unittest.main()
