#!/usr/bin/env python3
"""Offline unit tests for synthetic Ogg audio selection, discrete layout, and prefix budget fixtures."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

import generate_ogg_audio_selection_budget_regressions as gen


class OggAudioSelectionBudgetFixtureTests(unittest.TestCase):
    def test_generated_fixtures_match_committed_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            generated = gen.generate_fixtures(pathlib.Path(tmp))
            for key, path in generated.items():
                committed = gen.FIXTURES_DIR / path.name
                self.assertTrue(committed.exists(), f"missing committed fixture for {key}: {committed}")
                self.assertEqual(path.read_bytes(), committed.read_bytes())

    def test_ogg_pages_have_valid_magic_and_crc(self) -> None:
        for builder in (
            gen.build_skeleton_plus_opus,
            gen.build_discrete_family255_stereo_opus,
            gen.build_prefix_budget_opus,
        ):
            blob = builder()
            offset = 0
            pages = 0
            while offset < len(blob):
                self.assertEqual(blob[offset : offset + 4], b"OggS")
                seg_count = blob[offset + 26]
                table = blob[offset + 27 : offset + 27 + seg_count]
                body_len = sum(table)
                end = offset + 27 + seg_count + body_len
                page = bytearray(blob[offset:end])
                stored_crc = int.from_bytes(page[22:26], "little")
                page[22:26] = b"\x00\x00\x00\x00"
                self.assertEqual(stored_crc, gen.ogg_crc(bytes(page)))
                offset = end
                pages += 1
            self.assertGreaterEqual(pages, 4)


if __name__ == "__main__":
    unittest.main()
