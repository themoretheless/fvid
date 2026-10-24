#!/usr/bin/env python3
"""Offline unit tests for FLAC STREAMINFO and metadata regression fixtures (FVID-0194)."""

from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

import generate_flac_streaminfo_regressions as gen


class FlacStreaminfoRegressionFixtureTests(unittest.TestCase):
    def test_fixtures_are_byte_identical_to_generator(self) -> None:
        expected = gen.build_fixtures()
        for name, data in expected.items():
            disk_path = gen.FIXTURES_DIR / name
            self.assertTrue(disk_path.is_file(), f"missing fixture {disk_path}")
            self.assertEqual(disk_path.read_bytes(), data, f"stale fixture {name}")

    def test_flac_metadata_and_streaminfo_fault_isolation(self) -> None:
        fixtures = gen.build_fixtures()
        valid = fixtures["valid_baseline.flac"]
        self.assertEqual(valid[:4], b"fLaC")
        self.assertEqual(valid[4] & 0x7F, 0)
        self.assertEqual(int.from_bytes(valid[5:8], "big"), 34)

        pad_first = fixtures["padding_before_streaminfo.flac"]
        self.assertEqual(pad_first[4] & 0x7F, 1)

        overlong = fixtures["overlong_streaminfo.flac"]
        self.assertEqual(overlong[4] & 0x7F, 0)
        self.assertEqual(int.from_bytes(overlong[5:8], "big"), 36)

        min_below = fixtures["min_block_below_16.flac"]
        self.assertEqual(struct.unpack_from(">HH", min_below, 8), (15, 192))

        min_gt_max = fixtures["min_block_exceeds_max_block.flac"]
        self.assertEqual(struct.unpack_from(">HH", min_gt_max, 8), (256, 192))

        max_frame_bad = fixtures["max_frame_mismatch_when_min_zero.flac"]
        self.assertEqual(int.from_bytes(max_frame_bad[12:15], "big"), 0)
        self.assertEqual(int.from_bytes(max_frame_bad[15:18], "big"), 256)

    def test_write_fixtures_roundtrip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            written = gen.write_fixtures(Path(tmp))
            self.assertEqual(len(written), 12)
            self.assertTrue((Path(tmp) / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
