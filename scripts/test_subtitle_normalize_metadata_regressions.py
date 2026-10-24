#!/usr/bin/env python3
"""Offline unit test for synthetic subtitle conversion & normalize-loudness metadata fixtures."""

import pathlib
import struct
import unittest


class SubtitleNormalizeMetadataFixturesTest(unittest.TestCase):
    def test_fixtures_are_valid_and_deterministic(self) -> None:
        root = pathlib.Path(__file__).resolve().parents[1]
        fix_dir = root / "tests" / "fixtures" / "playback-errors" / "subtitle-normalize-metadata"
        srt = (fix_dir / "cues-two.srt").read_text(encoding="utf-8")
        self.assertIn("00:00:00,100 --> 00:00:01,100", srt)
        self.assertIn("00:00:01,200 --> 00:00:02,200", srt)

        ass = (fix_dir / "cues-two.ass").read_text(encoding="utf-8")
        self.assertIn("[Events]", ass)
        self.assertEqual(ass.count("Dialogue:"), 2)

        wav = (fix_dir / "tone-1s-48k.wav").read_bytes()
        self.assertEqual(wav[:4], b"RIFF")
        self.assertEqual(wav[8:12], b"WAVE")
        self.assertIn(b"LIST", wav)
        self.assertIn(b"Original Wave Title", wav)
        riff_size = struct.unpack("<I", wav[4:8])[0]
        self.assertEqual(riff_size + 8, len(wav))


if __name__ == "__main__":
    unittest.main()
