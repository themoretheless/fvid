#!/usr/bin/env python3
"""Unit test for FVID-0272 MP4-to-Matroska encoder tag and concat prefix fixture generator."""

from __future__ import annotations

import hashlib
import json
import unittest

from generate_mp4_matroska_encoder_concat_prefix import OUT, generate


class TestMp4MatroskaEncoderConcatPrefixFixtures(unittest.TestCase):
    def test_committed_fixtures_match_generator_and_manifest(self) -> None:
        expected = generate()
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0272")
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data, f"fixture {name} out of date")
            if name != "manifest.json":
                self.assertEqual(
                    manifest["sha256"][name],
                    hashlib.sha256(data).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
