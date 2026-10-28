#!/usr/bin/env python3
"""Unit tests for scripts/generate_mp4_audio_max_packets_budget.py."""

from __future__ import annotations

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_mp4_audio_max_packets_budget as gen


class GenerateMp4AudioMaxPacketsBudgetTest(unittest.TestCase):
    def test_deterministic_generation_matches_committed_fixtures(self) -> None:
        generated = gen.generate()
        self.assertEqual(
            sorted(generated.keys()),
            [
                "manifest.json",
                "prefix-elst-sowt.m4a",
                "prefix-sowt.m4a",
            ],
        )
        for name, expected in generated.items():
            committed = (gen.OUT / name).read_bytes()
            self.assertEqual(committed, expected, f"fixture mismatch for {name}")

    def test_mp4_header_present(self) -> None:
        generated = gen.generate()
        self.assertEqual(generated["prefix-sowt.m4a"][4:12], b"ftypisom")


if __name__ == "__main__":
    unittest.main()
