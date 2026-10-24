#!/usr/bin/env python3
"""Unit tests for the synthetic player queue/playlist fixture generator."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from generate_player_queue_regressions import build_m3u, build_pls, generate


class PlayerQueueGeneratorTests(unittest.TestCase):
    def test_pls_contains_section_header_and_quoted_entry(self) -> None:
        text = build_pls().decode("utf-8")
        self.assertIn("[playlist]", text)
        self.assertIn('File1="clip.avi"', text)
        self.assertIn("File2=clip.wv", text)

    def test_m3u_contains_quoted_and_plain_entries(self) -> None:
        text = build_m3u().decode("utf-8")
        self.assertTrue(text.startswith("#EXTM3U\n"))
        self.assertIn('"clip.avi"', text)
        self.assertIn("clip.wv", text)

    def test_generate_writes_expected_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            files = generate(Path(tmp))
            self.assertEqual(
                set(files.keys()),
                {"queue.pls", "queue.m3u", "clip.avi", "clip.wv", "companion.wvc"},
            )
            for path in files.values():
                self.assertTrue(path.is_file())


if __name__ == "__main__":
    unittest.main()
