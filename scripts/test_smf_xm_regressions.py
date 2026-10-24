#!/usr/bin/env python3
"""Offline unit tests for synthetic SMF and XM regression fixtures."""

from __future__ import annotations

import json
import struct
import tempfile
import unittest
from pathlib import Path

from generate_smf_xm_regressions import OUT_DIR, generate


class SmfXmRegressionsTest(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = generate(Path(tmp))
            for name, expected in generated.items():
                committed = (OUT_DIR / name).read_bytes()
                self.assertEqual(committed, expected, f"fixture mismatch for {name}")

    def test_smf_track_end_markers(self) -> None:
        valid = (OUT_DIR / "valid.mid").read_bytes()
        missing = (OUT_DIR / "missing-eot.mid").read_bytes()

        self.assertTrue(valid.endswith(b"\xff\x2f\x00"))
        self.assertNotIn(b"\xff\x2f", missing)

    def test_xm_unpacked_note_and_odd_16bit_sample(self) -> None:
        packed = (OUT_DIR / "packed-note-120.xm").read_bytes()
        unpacked = (OUT_DIR / "unpacked-note-120.xm").read_bytes()
        odd16 = (OUT_DIR / "odd-16bit-sample.xm").read_bytes()
        overrun = (OUT_DIR / "pattern-overrun-into-instrument.xm").read_bytes()

        # Pattern header starts at offset 60 + 276 = 336, length 9 -> pattern data at 345
        self.assertEqual(packed[345:351], bytes([0x9F, 120, 1, 0x40, 0, 0]))
        self.assertEqual(unpacked[345:350], bytes([120, 1, 0x40, 0, 0]))

        # Declared packed size in pattern-overrun-into-instrument.xm is 2 bytes
        packed_size = struct.unpack_from("<H", overrun, 336 + 7)[0]
        self.assertEqual(packed_size, 2)

        # Sample header in odd-16bit-sample.xm starts after pattern (345 + 6 = 351) + instrument (243) = 594
        sample_len = struct.unpack_from("<I", odd16, 594)[0]
        sample_type = odd16[594 + 14]
        self.assertEqual(sample_len, 5)
        self.assertTrue(sample_type & 0x10)

        manifest = json.loads((OUT_DIR / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(len(manifest["cases"]), 6)


if __name__ == "__main__":
    unittest.main()
