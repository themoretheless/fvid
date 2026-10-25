#!/usr/bin/env python3
"""Unit test for FVID-0274 MP4 remux prefix & stream budget fixture generator."""

from __future__ import annotations

import json
from pathlib import Path
import unittest

from generate_mp4_remux_prefix_budget import OUT, generate


class Mp4RemuxPrefixBudgetFixtureTest(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0274")


if __name__ == "__main__":
    unittest.main()
