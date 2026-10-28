#!/usr/bin/env python3
"""Offline unit test for Batch 256 (`FVID-0263`) fixtures."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
DIR = ROOT / "tests/fixtures/playback-errors/y4m-xfade-subburn-budget-parity"


class Y4mXfadeSubburnBudgetParityFixturesTest(unittest.TestCase):
    def test_manifest_hashes_and_headers(self) -> self:
        manifest_path = DIR / "manifest.json"
        self.assertTrue(manifest_path.is_file(), "manifest.json must exist")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0263")
        expected_files = {
            "multitrack-ffv1-aac.mkv",
            "multitrack-avc-aac.mp4",
            "seg-4x4-25fps-par1x1.y4m",
            "seg-4x4-50fps-par1x1.y4m",
            "seg-4x4-25fps-par2x1.y4m",
            "seg-4x4-25fps-par2x2.y4m",
            "seg-2x4-25fps-par1x1.y4m",
            "cues.srt",
        }
        self.assertEqual(set(manifest["sha256"].keys()), expected_files)
        for name, expected_hash in manifest["sha256"].items():
            path = DIR / name
            self.assertTrue(path.is_file(), f"missing fixture {name}")
            data = path.read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), expected_hash, f"hash mismatch for {name}")
            if name.endswith(".mkv"):
                self.assertTrue(data.startswith(b"\x1a\x45\xdf\xa3"))
            elif name.endswith(".mp4"):
                self.assertEqual(data[4:8], b"ftyp")
            elif name.endswith(".y4m"):
                self.assertTrue(data.startswith(b"YUV4MPEG2 "))


if __name__ == "__main__":
    unittest.main()
