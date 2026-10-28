#!/usr/bin/env python3
"""Unit test for FVID-0273 sample-domain concat prefix packet-byte budget fixture generator."""

from __future__ import annotations

import hashlib
import json
import unittest

from generate_audio_concat_packet_prefix_budget import OUT, generate


class TestAudioConcatPacketPrefixBudgetFixtures(unittest.TestCase):
    def test_committed_fixtures_match_generator_and_manifest(self) -> None:
        expected = generate()
        manifest = json.loads(expected["manifest.json"].decode("utf-8"))
        self.assertEqual(manifest["batch_id"], "FVID-0273")
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
