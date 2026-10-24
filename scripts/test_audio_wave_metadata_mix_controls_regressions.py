#!/usr/bin/env python3
"""Offline unit test for generate_audio_wave_metadata_mix_controls_regressions.py (FVID-0220)."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_audio_wave_metadata_mix_controls_regressions import OUT, generate


class TestAudioWaveMetadataMixControlsFixtures(unittest.TestCase):
    def test_fixtures_are_deterministic_and_match_disk(self) -> None:
        generated = generate()
        manifest = json.loads(generated["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0220")
        for name, data in generated.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {name}")
            self.assertEqual(path.read_bytes(), data, f"fixture mismatch for {name}")


if __name__ == "__main__":
    unittest.main()
