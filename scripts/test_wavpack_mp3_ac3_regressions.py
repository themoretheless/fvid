#!/usr/bin/env python3
"""Offline unit tests for synthetic WavPack, MP3, and AC-3 regression fixtures."""

from __future__ import annotations

import json
import struct
import tempfile
import unittest
from pathlib import Path

from generate_wavpack_mp3_ac3_regressions import OUT_DIR, generate


class WavpackMp3Ac3RegressionsTest(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = generate(Path(tmp))
            for name, expected in generated.items():
                committed = (OUT_DIR / name).read_bytes()
                self.assertEqual(committed, expected, f"fixture mismatch for {name}")

    def test_ape_and_subblock_layouts(self) -> None:
        valid = (OUT_DIR / "valid.wv").read_bytes()
        no_flag = (OUT_DIR / "ape-header-no-flag.wv").read_bytes()
        no_footer = (OUT_DIR / "ape-footer-no-footer-flag.wv").read_bytes()
        trailing = (OUT_DIR / "trailing-subblock-byte.wv").read_bytes()

        self.assertEqual(valid[-32:-24], b"APETAGEX")
        self.assertEqual(struct.unpack_from("<I", no_footer, len(no_footer) - 12)[0], 0x40000000)
        # In ape-header-no-flag.wv, footer flags has 0x80000000 while header flags has only 0x20000000
        self.assertEqual(struct.unpack_from("<I", no_flag, len(no_flag) - 12)[0], 0x80000000)
        # WavPack block ck_size is odd when 1 trailing byte is appended to word-aligned metadata
        ck_size = struct.unpack_from("<I", trailing, 4)[0]
        self.assertEqual(ck_size % 2, 1)

        manifest = json.loads((OUT_DIR / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(len(manifest["cases"]), 4)


if __name__ == "__main__":
    unittest.main()
