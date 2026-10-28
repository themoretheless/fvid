#!/usr/bin/env python3
"""Unit tests for the synthetic lossless planner fixture generator."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
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
            self.assertEqual(
                set(files.keys()),
                {
                    "source-8x4.y4m",
                    "zero-frames-8x4.y4m",
                    "two-frames-8x4.y4m",
                    "default-fps-8x4.y4m",
                },
            )
            self.assertTrue(files["source-8x4.y4m"].is_file())
            self.assertTrue(files["two-frames-8x4.y4m"].is_file())
            self.assertTrue(files["default-fps-8x4.y4m"].is_file())
            self.assertEqual(
                files["zero-frames-8x4.y4m"].read_bytes(),
                b"YUV4MPEG2 W8 H4 F25:1 Ip A1:1 C420jpeg\n",
            )


if __name__ == "__main__":
    unittest.main()
