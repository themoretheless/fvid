#!/usr/bin/env python3
"""Offline unit tests for scripts/generate_wave_ogg_flac_plan_regressions.py."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_wave_ogg_flac_plan_regressions as gen


class WaveOggFlacPlanRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_on_disk_and_manifest(self) -> None:
        files = gen.generate()
        manifest = json.loads(files["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0208")
        for name, expected in files.items():
            disk_path = gen.OUT / name
            self.assertTrue(disk_path.is_file(), f"missing fixture {disk_path}")
            self.assertEqual(disk_path.read_bytes(), expected, f"mismatch for {name}")
            if name != "manifest.json":
                self.assertEqual(
                    manifest["sha256"][name],
                    hashlib.sha256(expected).hexdigest(),
                )

    def test_wave_padding_and_g711_headers(self) -> None:
        files = gen.generate()
        valid = files["wave-24in32-valid.wav"]
        bad = files["wave-24in32-nonzero-padding.wav"]
        self.assertEqual(len(valid), len(bad))
        self.assertNotEqual(valid, bad)
        # fmt header stayed identical (valid_bits=24, bits_per_sample=32); only sample 7 LSB differs.
        self.assertEqual(valid[:80], bad[:80])
        self.assertEqual(int.from_bytes(valid[34:36], "little"), 32)
        self.assertEqual(int.from_bytes(valid[38:40], "little"), 24)
        self.assertTrue(files["g711-alaw-48k-mono.wav"].startswith(b"RIFF"))
        self.assertTrue(files["ogg-vorbis-chained-8ch-0x73f.ogg"].startswith(b"OggS"))


if __name__ == "__main__":
    unittest.main()
