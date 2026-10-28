#!/usr/bin/env python3
"""Unit tests for remux/concat policy parity synthetic fixture generator."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_remux_concat_policy_parity_regressions import (
    build_matroska_pcm_mono_48khz,
    build_mp4_aac_mono_48khz,
    generate_fixtures,
)


class RemuxConcatPolicyParityGeneratorTest(unittest.TestCase):
    def test_builders_emit_expected_headers(self) -> None:
        mp4 = build_mp4_aac_mono_48khz()
        mka = build_matroska_pcm_mono_48khz()
        self.assertEqual(mp4[4:8], b"ftyp")
        self.assertIn(b"moov", mp4)
        self.assertEqual(mka[:4], b"\x1a\x45\xdf\xa3")

    def test_generate_fixtures_writes_files_and_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = generate_fixtures(Path(tmp))
            self.assertTrue(Path(paths["mp4_aac"]).is_file())
            self.assertTrue(Path(paths["matroska_pcm"]).is_file())
            self.assertTrue((Path(tmp) / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
