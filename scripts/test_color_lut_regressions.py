#!/usr/bin/env python3
"""Offline unit tests for synthetic colour LUT and HDR mastering display regression fixtures."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from generate_color_lut_regressions import (
    build_1d_lut_between_grid_spike_cube_text,
    build_3dl_nan_node_text,
    build_collinear_mdcv_payload,
    build_cube_inverted_domain_text,
    build_spi1d_inverted_from_text,
    build_zero_mdcv_payload,
    generate_fixtures,
)


class ColorLutRegressionFixtureTests(unittest.TestCase):
    def test_3dl_nan_node_fixture(self) -> None:
        text = build_3dl_nan_node_text()
        self.assertIn("NaN", text)
        self.assertEqual(len(text.strip().splitlines()), 9)

    def test_inverted_domain_fixtures(self) -> None:
        cube = build_cube_inverted_domain_text()
        self.assertIn("DOMAIN_MIN 1.0 0.0 0.0", cube)
        self.assertIn("DOMAIN_MAX 0.0 1.0 1.0", cube)
        spi1d = build_spi1d_inverted_from_text()
        self.assertIn("From 1.0 0.0", spi1d)

    def test_1d_lut_spike_fixture(self) -> None:
        cube = build_1d_lut_between_grid_spike_cube_text()
        lines = cube.strip().splitlines()
        self.assertEqual(lines[0], "LUT_1D_SIZE 33")
        self.assertEqual(len(lines), 34)
        # Index 1 has red = 1/32 + 0.5 = 0.531250
        self.assertTrue(lines[2].startswith("0.531250 "))

    def test_mdcv_fixtures(self) -> None:
        zero = build_zero_mdcv_payload()
        self.assertEqual(len(zero), 24)
        self.assertEqual(zero, b"\x00" * 24)
        collinear = build_collinear_mdcv_payload()
        self.assertEqual(len(collinear), 24)

    def test_generate_fixtures_writes_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp)
            written = generate_fixtures(out)
            self.assertEqual(len(written), 6)
            self.assertTrue((out / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
