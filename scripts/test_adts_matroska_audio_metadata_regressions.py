#!/usr/bin/env python3
"""Offline unit test for generate_adts_matroska_audio_metadata_regressions.py."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

from generate_adts_matroska_audio_metadata_regressions import OUT_DIR, generate_all


class AdtsMatroskaAudioMetadataFixtureTest(unittest.TestCase):
    def test_generated_adts_and_wave_fixtures_are_deterministic_and_match_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = generate_all(pathlib.Path(tmp))
            self.assertEqual(
                set(generated.keys()),
                {"adts-seg-a-48k.aac", "adts-seg-b-48k.aac", "pcm-source-48k.wav"},
            )
            for name, payload in generated.items():
                if name.endswith(".aac"):
                    self.assertEqual(payload[0], 0xFF)
                    self.assertEqual(payload[1] & 0xF0, 0xF0)
                else:
                    self.assertTrue(payload.startswith(b"RIFF"))
                committed = (OUT_DIR / name).read_bytes()
                self.assertEqual(payload, committed, f"fixture mismatch for {name}")


if __name__ == "__main__":
    unittest.main()
