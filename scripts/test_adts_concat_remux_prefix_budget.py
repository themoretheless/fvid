#!/usr/bin/env python3
import tempfile
import unittest
from pathlib import Path
import sys

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from generate_adts_concat_remux_prefix_budget import generate_fixtures


class TestAdtsConcatRemuxPrefixBudgetFixtures(unittest.TestCase):
    def test_deterministic_generation_and_sizes(self) -> None:
        repo_root = SCRIPTS_DIR.parent
        committed_dir = (
            repo_root / "tests" / "fixtures" / "playback-errors" / "adts-concat-remux-prefix-budget"
        )
        with tempfile.TemporaryDirectory() as tmp:
            tmp_dir = Path(tmp)
            manifest = generate_fixtures(tmp_dir)
            self.assertEqual(
                set(manifest["files"].keys()),
                {
                    "seg-a-small.aac",
                    "seg-b-large.aac",
                    "seg-c-invalid-rate.aac",
                    "sbr-24k-to-48k.aac",
                },
            )
            for name in manifest["files"]:
                self.assertEqual(
                    (tmp_dir / name).read_bytes(),
                    (committed_dir / name).read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
