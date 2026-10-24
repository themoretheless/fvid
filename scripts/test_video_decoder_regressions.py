#!/usr/bin/env python3
"""Offline unit tests for synthetic video decoder regression fixtures."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_video_decoder_regressions as gen


class VideoDecoderRegressionFixturesTest(unittest.TestCase):
    def test_generated_files_match_committed_fixtures(self) -> None:
        root = pathlib.Path(__file__).resolve().parents[1]
        committed = root / "tests" / "fixtures" / "playback-errors" / "video-decoders"
        with tempfile.TemporaryDirectory() as tmp:
            files = gen.generate(pathlib.Path(tmp))
            self.assertEqual(len(files), 4)
            for name, data in files.items():
                path = committed / name
                self.assertTrue(path.is_file(), f"missing committed fixture: {name}")
                self.assertEqual(path.read_bytes(), data, f"fixture mismatch: {name}")

    def test_ffv1_v3_config_crc_and_packets(self) -> None:
        cfg = gen.build_ffv1_v3_config()
        self.assertGreaterEqual(len(cfg), 6)
        self.assertEqual(gen.ffv1_crc(cfg), 0)

        kf = gen.build_ffv1_v3_slice_packet(True, 0, [0, 0, 0, 10])
        inter = gen.build_ffv1_v3_slice_packet(False, 1, [0, 0, 0, 42])
        self.assertEqual(int.from_bytes(kf[-3:], "big"), len(kf) - 3)
        self.assertEqual(int.from_bytes(inter[-3:], "big"), len(inter) - 3)


if __name__ == "__main__":
    unittest.main()
