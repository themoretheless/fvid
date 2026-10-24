#!/usr/bin/env python3
"""Offline unit test for playback video reader regression fixtures."""
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_video_reader_regressions as gen


class VideoReaderRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_disk(self):
        expected = gen.generate()
        for name, data in expected.items():
            path = gen.OUT / name
            self.assertTrue(path.is_file(), f"missing {path}")
            self.assertEqual(path.read_bytes(), data, f"stale {name}")

    def test_manifest_hashes_are_consistent(self):
        manifest = json.loads((gen.OUT / "manifest.json").read_text())
        self.assertEqual(len(manifest["fixtures"]), 3)
        for item in manifest["fixtures"]:
            raw = (gen.OUT / item["file"]).read_bytes()
            self.assertEqual(len(raw), item["size"])
            self.assertEqual(hashlib.sha256(raw).hexdigest(), item["sha256"])


if __name__ == "__main__":
    unittest.main()
