#!/usr/bin/env python3
"""Unit tests for the FVID-0257 concat and trim packet-byte budget and error parity fixture generator."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from generate_concat_trim_budget_parity_regressions import (
    OUT,
    build_mp4_aac_mono_bytes,
    build_ogg_opus_mono_bytes,
    build_wave_stereo_f32_bytes,
    build_wave_stereo_s16_bytes,
    generate,
)


class ConcatTrimBudgetParityFixtureGeneratorTests(unittest.TestCase):
    def test_builders_are_deterministic_and_compact(self) -> None:
        w16 = build_wave_stereo_s16_bytes()
        wf32 = build_wave_stereo_f32_bytes()
        ogg = build_ogg_opus_mono_bytes()
        mp4 = build_mp4_aac_mono_bytes()

        self.assertEqual(w16, build_wave_stereo_s16_bytes())
        self.assertEqual(wf32, build_wave_stereo_f32_bytes())
        self.assertEqual(ogg, build_ogg_opus_mono_bytes())
        self.assertEqual(mp4, build_mp4_aac_mono_bytes())

        self.assertTrue(w16.startswith(b"RIFF"))
        self.assertTrue(wf32.startswith(b"RIFF"))
        self.assertTrue(ogg.startswith(b"OggS"))
        self.assertEqual(mp4[4:8], b"ftyp")

    def test_committed_fixtures_match_generator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            manifest = generate(Path(tmp))
            self.assertEqual(len(manifest["fixtures"]), 4)
            for item in manifest["fixtures"]:
                committed = OUT / item["name"]
                self.assertTrue(committed.is_file(), f"missing {committed}")
                self.assertEqual(
                    committed.read_bytes(),
                    (Path(tmp) / item["name"]).read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
