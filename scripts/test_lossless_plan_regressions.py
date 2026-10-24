#!/usr/bin/env python3
"""Unit tests for the synthetic lossless planner fixture generator."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from generate_lossless_plan_regressions import build_y4m, generate


class LosslessPlanGeneratorTests(unittest.TestCase):
    def test_build_y4m_has_exact_header_and_frame_size(self) -> None:
        raw = build_y4m(8, 4)
        header, rest = raw.split(b"\n", 1)
        self.assertEqual(header, b"YUV4MPEG2 W8 H4 F25:1 Ip A1:1 C420jpeg")
        self.assertTrue(rest.startswith(b"FRAME\n"))
        payload = rest[len(b"FRAME\n") :]
        self.assertEqual(len(payload), 8 * 4 + 2 * (4 * 2))

    def test_generate_writes_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            files = generate(Path(tmp))
            self.assertEqual(set(files.keys()), {"source-8x4.y4m"})
            self.assertTrue(files["source-8x4.y4m"].is_file())


if __name__ == "__main__":
    unittest.main()
