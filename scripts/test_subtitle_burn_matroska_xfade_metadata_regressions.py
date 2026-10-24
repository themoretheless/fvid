#!/usr/bin/env python3
"""Offline unit test for `generate_subtitle_burn_matroska_xfade_metadata_regressions.py`."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_subtitle_burn_matroska_xfade_metadata_regressions as gen


class SubtitleBurnMatroskaXfadeMetadataGeneratorTests(unittest.TestCase):
    def test_generator_matches_committed_fixtures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = gen.generate_all(Path(tmp))
        self.assertTrue(gen.OUT.is_dir(), f"missing fixture dir {gen.OUT}")
        manifest = json.loads((gen.OUT / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0239")
        for name, expected in generated.items():
            committed = (gen.OUT / name).read_bytes()
            self.assertEqual(committed, expected, f"fixture mismatch for {name}")
            if name != "manifest.json":
                self.assertEqual(
                    manifest["files"][name]["sha256"],
                    hashlib.sha256(committed).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
