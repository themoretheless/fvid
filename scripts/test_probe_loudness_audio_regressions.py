#!/usr/bin/env python3
"""Offline unit tests for scripts/generate_probe_loudness_audio_regressions.py."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_probe_loudness_audio_regressions as gen


class ProbeLoudnessAudioRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_on_disk_and_manifest(self) -> None:
        files = gen.generate()
        manifest = json.loads(files["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0209")
        for name, expected in files.items():
            disk_path = gen.OUT / name
            self.assertTrue(disk_path.is_file(), f"missing fixture {disk_path}")
            self.assertEqual(disk_path.read_bytes(), expected, f"mismatch for {name}")
            if name != "manifest.json":
                self.assertEqual(
                    manifest["sha256"][name],
                    hashlib.sha256(expected).hexdigest(),
                )

    def test_lame_info_frame_crc_and_trim_headers(self) -> None:
        files = gen.generate()
        empty_mp3 = files["raw-mp3-empty-trimmed.mp3"]
        self.assertEqual(empty_mp3[36:40], b"Info")
        self.assertEqual(empty_mp3[48:57], b"LAME3.100")
        stored_crc = int.from_bytes(empty_mp3[82:84], "big")
        self.assertEqual(gen.mp3_crc16(empty_mp3[:82]), stored_crc)
        self.assertTrue(files["g711-alaw-48k.wav"].startswith(b"RIFF"))
        self.assertTrue(files["ogg-opus-48k.ogg"].startswith(b"OggS"))
        self.assertTrue(files["raw-flac-48k.flac"].startswith(b"fLaC"))


if __name__ == "__main__":
    unittest.main()
