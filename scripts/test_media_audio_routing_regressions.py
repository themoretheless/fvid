#!/usr/bin/env python3
"""Offline unit tests for media-audio-routing synthetic fixtures."""

from __future__ import annotations

import hashlib
import json
import unittest

from generate_media_audio_routing_regressions import OUT, generate


class MediaAudioRoutingFixtureTests(unittest.TestCase):
    def test_committed_fixtures_match_generator(self) -> None:
        expected = generate()
        manifest = json.loads((OUT / "manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest.get("batch_id"), "FVID-0210")
        for name, data in expected.items():
            path = OUT / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")
            self.assertEqual(path.read_bytes(), data)
            if name != "manifest.json":
                self.assertEqual(
                    manifest["sha256"][name],
                    hashlib.sha256(data).hexdigest(),
                )


if __name__ == "__main__":
    unittest.main()
