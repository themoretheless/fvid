#!/usr/bin/env python3
"""Unit tests for scripts/generate_multitrack_lossless_budget_parity_regressions.py."""

from __future__ import annotations

from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_multitrack_lossless_budget_parity_regressions import generate


class GenerateMultitrackLosslessBudgetParityRegressionsTests(unittest.TestCase):
    def test_generates_deterministic_multitrack_fixtures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            files = generate(Path(tmp))
            self.assertIn("multitrack-ffv1-aac.mkv", files)
            self.assertIn("multitrack-avc-aac.mp4", files)
            self.assertTrue(files["multitrack-ffv1-aac.mkv"].startswith(b"\x1a\x45\xdf\xa3"))
            self.assertEqual(files["multitrack-avc-aac.mp4"][4:8], b"ftyp")


if __name__ == "__main__":
    unittest.main()
