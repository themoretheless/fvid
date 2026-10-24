#!/usr/bin/env python3
"""Offline unit test verifying synthetic loudnorm Matroska output and metadata fixtures for FVID-0231."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_loudnorm_matroska_metadata_regressions import OUT, generate


class LoudnormMatroskaMetadataRegressionFixtureTest(unittest.TestCase):
    def test_generated_files_match_committed_fixtures_and_manifest(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")

        manifest = json.loads((OUT / "manifest.json").read_text("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0231")
        for name, digest in manifest["sha256"].items():
            actual = hashlib.sha256((OUT / name).read_bytes()).hexdigest()
            self.assertEqual(actual, digest, f"sha256 mismatch for {name}")

    def test_fixture_headers(self) -> None:
        wav = (OUT / "tone-tagged-48k.wav").read_bytes()
        self.assertTrue(wav.startswith(b"RIFF"))
        self.assertIn(b"Source Wave Loudnorm", wav)

        aac = (OUT / "tone-48k.aac").read_bytes()
        self.assertEqual(aac[0], 0xFF)
        self.assertEqual(aac[1] & 0xF0, 0xF0)

        mp3 = (OUT / "tone-tagged-48k.mp3").read_bytes()
        self.assertTrue(mp3.startswith(b"ID3\x04\x00\x00"))


if __name__ == "__main__":
    unittest.main()
