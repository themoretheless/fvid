#!/usr/bin/env python3
"""Unit tests for the FVID-0256 Ogg Opus, G.711 WAVE, and ADTS loudnorm budget/stream-selection fixture generator."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from generate_ogg_g711_loudnorm_budget_regressions import (
    OUT,
    build_adts_mono_bytes,
    build_g711_wave_stereo_alaw_bytes,
    build_ogg_opus_mono_bytes,
    generate,
)


class OggG711LoudnormBudgetFixtureGeneratorTests(unittest.TestCase):
    def test_builders_are_deterministic_and_compact(self) -> None:
        ogg = build_ogg_opus_mono_bytes()
        g711 = build_g711_wave_stereo_alaw_bytes()
        adts = build_adts_mono_bytes()

        self.assertEqual(ogg, build_ogg_opus_mono_bytes())
        self.assertEqual(g711, build_g711_wave_stereo_alaw_bytes())
        self.assertEqual(adts, build_adts_mono_bytes())

        self.assertTrue(ogg.startswith(b"OggS"))
        self.assertTrue(g711.startswith(b"RIFF"))
        self.assertEqual(adts[0], 0xFF)
        self.assertLess(len(ogg), 256)
        self.assertLess(len(g711), 128)
        self.assertEqual(len(adts), 13)

    def test_committed_fixtures_match_generator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest = generate(Path(tmp))
            self.assertEqual(len(manifest["fixtures"]), 3)
            for item in manifest["fixtures"]:
                committed = OUT / item["name"]
                self.assertTrue(committed.is_file(), f"missing {committed}")
                self.assertEqual(
                    committed.read_bytes(),
                    (Path(tmp) / item["name"]).read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
