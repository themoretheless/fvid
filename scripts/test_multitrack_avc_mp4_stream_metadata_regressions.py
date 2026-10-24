#!/usr/bin/env python3
"""Offline unit test for Batch 233 (`FVID-0240`) synthetic fixtures."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_multitrack_avc_mp4_stream_metadata_regressions import OUT, generate


class MultitrackAvcMp4StreamMetadataFixtureTests(unittest.TestCase):
    def test_generated_fixtures_match_committed_bytes_and_manifest(self) -> None:
        expected = generate()
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0240")
        for name, content in expected.items():
            path = OUT / name
            self.assertTrue(path.exists(), f"missing fixture {path}")
            actual = path.read_bytes()
            self.assertEqual(actual, content, f"fixture drift in {name}")
            if name != "manifest.json":
                entry = manifest["files"][name]
                self.assertEqual(entry["bytes"], len(actual))
                self.assertEqual(
                    entry["sha256"],
                    hashlib.sha256(actual).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
