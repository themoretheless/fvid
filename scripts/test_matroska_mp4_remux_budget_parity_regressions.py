#!/usr/bin/env python3
"""Unit tests for `generate_matroska_mp4_remux_budget_parity_regressions.py`."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_matroska_mp4_remux_budget_parity_regressions import OUT, generate_all


class MatroskaMp4RemuxBudgetParityFixturesTests(unittest.TestCase):
    def test_generated_fixtures_match_committed_files_and_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            generated = generate_all(tmp_dir)
            self.assertEqual(
                set(generated.keys()),
                {
                    "matroska-aac-48k.mka",
                    "mp4-aac-48k.m4a",
                    "mp4-pcm-48k.m4a",
                },
            )
            manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["batch"], 240)
            self.assertEqual(manifest["id"], "FVID-0247")
            by_name = {entry["name"]: entry for entry in manifest["fixtures"]}
            for name, data in generated.items():
                committed = (OUT / name).read_bytes()
                self.assertEqual(data, committed, f"mismatch for {name}")
                self.assertEqual(by_name[name]["bytes"], len(data))
                self.assertEqual(
                    by_name[name]["sha256"],
                    hashlib.sha256(data).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
