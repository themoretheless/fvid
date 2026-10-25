#!/usr/bin/env python3
"""Offline unit tests for y4m-prefix-budget synthetic fixtures."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_y4m_prefix_budget import OUT, generate


class Y4mPrefixBudgetFixtureTests(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        expected = generate()
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest.get("batch_id"), "FVID-0275")
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data)
            if name != "manifest.json":
                self.assertEqual(
                    manifest["sha256"][name],
                    hashlib.sha256(data).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
