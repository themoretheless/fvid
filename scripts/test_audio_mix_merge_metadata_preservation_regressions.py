#!/usr/bin/env python3
"""Offline unit test for FVID-0236 synthetic fixtures."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_audio_mix_merge_metadata_preservation_regressions import OUT, generate


class TestAudioMixMergeMetadataPreservationFixtures(unittest.TestCase):
    def test_generated_fixtures_match_disk_and_manifest(self) -> None:
        expected = generate()
        self.assertTrue(OUT.is_dir(), f"missing fixture directory {OUT}")
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {name}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0236")
        for name, sha in manifest["sha256"].items():
            self.assertEqual(hashlib.sha256((OUT / name).read_bytes()).hexdigest(), sha)


if __name__ == "__main__":
    unittest.main()
