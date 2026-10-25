#!/usr/bin/env python3
"""Unit tests for synthetic bare FLAC and bare MP3 prefix budget fixtures (FVID-0266)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import unittest

from generate_raw_flac_mp3_prefix_budget_regressions import OUT, generate


class RawFlacMp3PrefixBudgetFixtureTests(unittest.TestCase):
    def test_saved_fixtures_match_generator_and_manifest(self) -> None:
        expected = generate()
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0266")
        for name, content in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            saved = path.read_bytes()
            self.assertEqual(saved, content, f"fixture mismatch for {name}")
            if name != "manifest.json":
                self.assertEqual(
                    hashlib.sha256(saved).hexdigest(),
                    manifest["sha256"][name],
                )

    def test_frame_sizes_span_prefix_budget_boundary(self) -> None:
        files = generate()
        self.assertTrue(files["prefix-budget.flac"].startswith(b"fLaC"))
        self.assertEqual(len(files["prefix-budget.flac"]), 4 + 4 + 34 + 11 + 393)
        self.assertEqual(len(files["prefix-budget.mp3"]), 960 + 961 + 960 + 961)


if __name__ == "__main__":
    unittest.main()
