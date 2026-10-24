#!/usr/bin/env python3
"""Unit tests for Batch 237 (`FVID-0244`) fixture generator."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_remux_concat_wave_metadata_regressions as gen


class RemuxConcatWaveMetadataFixtureTests(unittest.TestCase):
    def test_generated_fixtures_match_disk_and_manifest(self) -> None:
        files = gen.generate()
        manifest = json.loads(files["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0244")
        for name, data in files.items():
            path = gen.OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"stale fixture {path}")


if __name__ == "__main__":
    unittest.main()
