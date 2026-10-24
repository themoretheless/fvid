#!/usr/bin/env python3
"""Offline unit tests for synthetic audio decoder regression fixtures."""

from __future__ import annotations

import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_audio_decoder_regressions import OUT_DIR, generate


class AudioDecoderRegressionsTest(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = generate(Path(tmp))
            for name, expected in generated.items():
                committed = (OUT_DIR / name).read_bytes()
                self.assertEqual(committed, expected, f"fixture mismatch for {name}")

    def test_flac_and_xm_fixture_invariants(self) -> None:
        si_64 = (OUT_DIR / "flac_streaminfo_max_64.bin").read_bytes()
        min_block, max_block = struct.unpack_from(">HH", si_64, 0)
        self.assertEqual((min_block, max_block), (16, 64))

        si_frame_bounds = (OUT_DIR / "flac_streaminfo_min_frame_gt_max_frame.bin").read_bytes()
        min_frame = int.from_bytes(si_frame_bounds[4:7], "big")
        max_frame = int.from_bytes(si_frame_bounds[7:10], "big")
        self.assertGreater(min_frame, max_frame)

        row_1ch = (OUT_DIR / "xm_row_1ch_mismatch.bin").read_bytes()
        row_2ch = (OUT_DIR / "xm_row_2ch_valid.bin").read_bytes()
        self.assertEqual(len(row_1ch), 8 + 5)
        self.assertEqual(len(row_2ch), 8 + 10)

        manifest = json.loads((OUT_DIR / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(len(manifest["cases"]), 5)


if __name__ == "__main__":
    unittest.main()
