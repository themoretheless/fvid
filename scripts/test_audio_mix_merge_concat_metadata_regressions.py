#!/usr/bin/env python3
"""Offline unit test for generate_audio_mix_merge_concat_metadata_regressions.py."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

from generate_audio_mix_merge_concat_metadata_regressions import OUT_DIR, generate_all


class AudioMixMergeConcatMetadataFixtureTest(unittest.TestCase):
    def test_generated_wave_fixtures_are_deterministic_and_match_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = generate_all(pathlib.Path(tmp))
            self.assertEqual(
                set(generated.keys()),
                {"tone-a-48k-stereo.wav", "tone-b-48k-stereo.wav"},
            )
            for name, payload in generated.items():
                self.assertTrue(payload.startswith(b"RIFF"))
                self.assertIn(b"LIST", payload)
                self.assertIn(b"INFO", payload)
                committed = (OUT_DIR / name).read_bytes()
                self.assertEqual(payload, committed, f"fixture mismatch for {name}")


if __name__ == "__main__":
    unittest.main()
