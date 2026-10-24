#!/usr/bin/env python3
"""Offline unit test verifying synthetic bare FLAC and Ogg Vorbis/Opus metadata fixtures for FVID-0229."""

from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_flac_ogg_metadata_regressions import OUT, generate


class FlacOggMetadataRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_disk(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")
        self.assertTrue(expected["tagged.flac"].startswith(b"fLaC"))
        self.assertTrue(expected["tagged-opus.ogg"].startswith(b"OggS"))
        self.assertTrue(expected["tagged-vorbis.ogg"].startswith(b"OggS"))


if __name__ == "__main__":
    unittest.main()
