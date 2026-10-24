#!/usr/bin/env python3
"""Offline unit test for synthetic Ogg framing and track-index regression fixtures."""
import unittest
from generate_ogg_framing_track_regressions import OUT, generate


class TestOggFramingTrackRegressions(unittest.TestCase):
    def test_fixtures_match_generator(self) -> None:
        expected = generate()
        for name, data in expected.items():
            self.assertEqual((OUT / name).read_bytes(), data, name)


if __name__ == "__main__":
    unittest.main()
