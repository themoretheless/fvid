#!/usr/bin/env python3
"""Offline unit test for scripts/generate_loudnorm_wave_metadata_routing_regressions.py (FVID-0221)."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_loudnorm_wave_metadata_routing_regressions as gen


class LoudnormWaveMetadataRoutingRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_disk_and_manifest(self) -> None:
        expected = gen.generate()
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0221")
        for name, data in expected.items():
            disk = (gen.OUT / name).read_bytes()
            self.assertEqual(disk, data, f"fixture mismatch for {name}")
            if name != "manifest.json":
                self.assertEqual(manifest["fixtures"][name]["bytes"], len(data))


if __name__ == "__main__":
    unittest.main()
