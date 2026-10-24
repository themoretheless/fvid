#!/usr/bin/env python3
"""Unit tests for `generate_adts_wave_remux_concat_parity_regressions.py`."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from generate_adts_wave_remux_concat_parity_regressions import OUT, generate_all


class AdtsWaveRemuxConcatParityFixturesTests(unittest.TestCase):
    def test_generated_fixtures_match_committed_files_and_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            generated = generate_all(tmp_dir)
            self.assertEqual(
                set(generated.keys()),
                {
                    "adts-seg-a.aac",
                    "adts-seg-b.aac",
                    "wave-seg-a.wav",
                    "wave-seg-b.wav",
                },
            )
            manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["batch"], 239)
            self.assertEqual(manifest["id"], "FVID-0246")
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
