#!/usr/bin/env python3
"""Offline unit test verifying synthetic bare MP3 ID3v2/ID3v1 metadata fixtures for FVID-0230."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import unittest

from generate_mp3_id3_metadata_regressions import OUT, generate


class Mp3Id3MetadataRegressionFixtureTest(unittest.TestCase):
    def test_generated_files_match_committed_fixtures_and_manifest(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")

        manifest = json.loads((OUT / "manifest.json").read_text("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0230")
        for name, digest in manifest["sha256"].items():
            actual = hashlib.sha256((OUT / name).read_bytes()).hexdigest()
            self.assertEqual(actual, digest, f"sha256 mismatch for {name}")

    def test_id3v24_and_id3v1_structure(self) -> None:
        v24 = (OUT / "tagged-id3v24.mp3").read_bytes()
        self.assertTrue(v24.startswith(b"ID3\x04\x00\x00"))
        self.assertEqual(v24[-128:-125], b"TAG")
        self.assertIn(b"TIT2", v24)
        self.assertIn(b"TPE1", v24)
        self.assertIn(b"COMM", v24)
        self.assertIn(b"TXXX", v24)

        v1 = (OUT / "tagged-id3v1.mp3").read_bytes()
        self.assertFalse(v1.startswith(b"ID3"))
        self.assertEqual(v1[-128:-125], b"TAG")
        self.assertIn(b"ID3v1 Fallback Title", v1[-128:])


if __name__ == "__main__":
    unittest.main()
