#!/usr/bin/env python3
"""Offline unit test for FVID-0227 synthetic MP4 and Y4M fixtures."""

from __future__ import annotations

import json
import unittest

from generate_mp4_remux_custom_tags_regressions import OUT, generate


class Mp4RemuxCustomTagsRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_disk_and_manifest(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0227")
        self.assertIn("two-track-avc.mp4", manifest["sha256"])
        self.assertIn("tiny-two-frames.y4m", manifest["sha256"])


if __name__ == "__main__":
    unittest.main()
