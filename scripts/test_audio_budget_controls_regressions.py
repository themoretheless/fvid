#!/usr/bin/env python3
"""Offline unit test verifying synthetic audio budget controls fixtures (FVID-0214)."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import unittest

from generate_audio_budget_controls_regressions import OUT, generate


class AudioBudgetControlsFixturesTest(unittest.TestCase):
    def test_fixtures_match_generator_and_manifest(self) -> None:
        manifest_path = OUT / "manifest.json"
        self.assertTrue(manifest_path.is_file(), f"missing {manifest_path}")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        self.assertEqual(manifest.get("batch_id"), "FVID-0214")
        expected = generate()
        self.assertEqual(set(manifest["sha256"].keys()), set(expected.keys()))
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            actual = path.read_bytes()
            self.assertEqual(actual, data, f"fixture content mismatch for {name}")
            digest = hashlib.sha256(actual).hexdigest()
            self.assertEqual(digest, manifest["sha256"][name], f"sha256 mismatch for {name}")


if __name__ == "__main__":
    unittest.main()
