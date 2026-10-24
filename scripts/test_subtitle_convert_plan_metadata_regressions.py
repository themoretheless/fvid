#!/usr/bin/env python3
"""Unit test for subtitle convert plan & WritingApp metadata preservation fixture generator (FVID-0237)."""

from __future__ import annotations

import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))

from generate_subtitle_convert_plan_metadata_regressions import OUT, generate


class TestSubtitleConvertPlanMetadataRegressions(unittest.TestCase):
    def test_generated_fixtures_match_disk(self) -> None:
        generated = generate()
        manifest = json.loads(generated["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0237")
        for name, expected in generated.items():
            path = OUT / name
            self.assertTrue(path.exists(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), expected, f"fixture mismatch for {name}")


if __name__ == "__main__":
    unittest.main()
