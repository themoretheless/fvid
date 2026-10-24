#!/usr/bin/env python3
import hashlib, json, sys, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_container_loudness_plan_regressions import OUT, generate


class ContainerLoudnessPlanRegressionsTest(unittest.TestCase):
    def test_generated_fixtures_match_saved_manifest(self):
        files, expected_manifest = generate()
        saved_manifest = json.loads((OUT / "manifest.json").read_text())
        self.assertEqual(saved_manifest, expected_manifest)
        for name, expected_bytes in files.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {name}")
            actual = path.read_bytes()
            self.assertEqual(actual, expected_bytes, f"content mismatch for {name}")
            self.assertEqual(
                hashlib.sha256(actual).hexdigest(),
                saved_manifest["files"][name]["sha256"],
            )


if __name__ == "__main__":
    unittest.main()
