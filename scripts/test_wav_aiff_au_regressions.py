#!/usr/bin/env python3
"""Offline unit tests for WAV/AIFF/AU synthetic regression fixtures (FVID-0193)."""

from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

import generate_wav_aiff_au_regressions as gen


class WavAiffAuRegressionFixtureTests(unittest.TestCase):
    def test_fixtures_are_byte_identical_to_generator(self) -> None:
        expected = gen.build_fixtures()
        for name, data in expected.items():
            disk_path = gen.FIXTURES_DIR / name
            self.assertTrue(disk_path.is_file(), f"missing fixture {disk_path}")
            self.assertEqual(disk_path.read_bytes(), data, f"stale fixture {name}")

    def test_wav_extensible_geometry_and_fault_isolation(self) -> None:
        fixtures = gen.build_fixtures()
        valid = fixtures["wav_extensible_valid.wav"]
        mask_bad = fixtures["wav_extensible_channel_mask_overflow.wav"]
        cb_bad = fixtures["wav_extensible_cbsize_overflow.wav"]

        self.assertEqual(len(valid), len(mask_bad))
        self.assertEqual(len(valid), len(cb_bad))
        # fmt chunk body begins at offset 20 (12-byte RIFF header + 8-byte fmt header)
        # cbSize is at fmt_body[16..18] -> file[36..38]
        # dwChannelMask is at fmt_body[20..24] -> file[40..44]
        self.assertEqual(struct.unpack_from("<H", valid, 36)[0], 22)
        self.assertEqual(struct.unpack_from("<I", valid, 40)[0], 0x04)
        self.assertEqual(struct.unpack_from("<I", mask_bad, 40)[0], 0x03)
        self.assertEqual(struct.unpack_from("<H", cb_bad, 36)[0], 30)

    def test_aifc_g711_sample_width_isolation(self) -> None:
        fixtures = gen.build_fixtures()
        valid = fixtures["aifc_alaw_8bit_valid.aifc"]
        bad16 = fixtures["aifc_alaw_16bit_invalid.aifc"]
        bad24 = fixtures["aifc_ulaw_24bit_invalid.aifc"]

        # COMM body begins at offset 32 (12 FORM + 12 FVER + 8 COMM header)
        # sampleSize is at COMM[6..8] -> file[38..40]
        self.assertEqual(struct.unpack_from(">h", valid, 38)[0], 8)
        self.assertEqual(struct.unpack_from(">h", bad16, 38)[0], 16)
        self.assertEqual(struct.unpack_from(">h", bad24, 38)[0], 24)

    def test_write_fixtures_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            written = gen.write_fixtures(Path(tmp))
            self.assertEqual(len(written), 7)
            self.assertTrue((Path(tmp) / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
