#!/usr/bin/env python3
"""Offline unit test for FVID-0280 synthetic Matroska interval-prefix fixtures."""

from __future__ import annotations

from pathlib import Path
import unittest

from generate_matroska_interval_prefix_budget import OUT, generate


class TestMatroskaIntervalPrefixBudgetFixtures(unittest.TestCase):
    def test_generated_files_match_saved_fixtures(self) -> None:
        expected = generate()
        for name, data in expected.items():
            path = Path(OUT) / name
            self.assertTrue(path.is_file(), f"missing fixture: {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch: {path}")


if __name__ == "__main__":
    unittest.main()
