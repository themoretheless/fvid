#!/usr/bin/env python3
"""Unit test for `scripts/generate_mp4_adts_interval_prefix_budget.py`."""

from __future__ import annotations

import json
from pathlib import Path
import tempfile
import unittest

from generate_mp4_adts_interval_prefix_budget import generate_fixtures


class Mp4AdtsIntervalPrefixBudgetFixtureTest(unittest.TestCase):
    def test_regenerates_identical_fixtures(self) -> None:
        repo_root = Path(__file__).resolve().parent.parent
        checked_in = (
            repo_root
            / "tests"
            / "fixtures"
            / "playback-errors"
            / "mp4-adts-interval-prefix-budget"
        )
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            manifest = generate_fixtures(tmp_dir)
            checked_manifest = json.loads((checked_in / "manifest.json").read_text())
            self.assertEqual(manifest, checked_manifest)
            for name in manifest["files"]:
                self.assertEqual(
                    (tmp_dir / name).read_bytes(),
                    (checked_in / name).read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
