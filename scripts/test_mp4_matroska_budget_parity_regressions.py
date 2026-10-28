#!/usr/bin/env python3
"""Unit tests for the MP4-to-Matroska budget parity synthetic fixture generator."""

from __future__ import annotations

import unittest

from generate_mp4_matroska_budget_parity_regressions import OUT, generate


class Mp4MatroskaBudgetParityFixtureTests(unittest.TestCase):
    def test_synthetic_payloads_have_valid_headers(self) -> None:
        files = generate()
        mp4 = files["avc-32x16.mp4"]
        self.assertEqual(mp4[4:8], b"ftyp")
        self.assertIn(b"moov", mp4)
        self.assertIn(b"mdat", mp4)

        wav = files["pcm-48k-mono.wav"]
        self.assertEqual(wav[:4], b"RIFF")
        self.assertEqual(wav[8:12], b"WAVE")

        adts = files["aac-48k-mono.aac"]
        self.assertEqual(adts[0], 0xFF)
        self.assertEqual(adts[1] & 0xF0, 0xF0)

    def test_manifest_matches_committed_fixture_manifest(self) -> None:
        files = generate()
        committed = (OUT / "manifest.json").read_bytes()
        self.assertEqual(committed, files["manifest.json"])


if __name__ == "__main__":
    unittest.main()
