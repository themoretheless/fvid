#!/usr/bin/env python3
"""Offline unit test for generate_lossless_plan_cli_routing_regressions.py."""

from __future__ import annotations

import pathlib
import tempfile
import unittest

from generate_lossless_plan_cli_routing_regressions import FIXTURES, generate


class LosslessPlanCliRoutingFixtureTest(unittest.TestCase):
    def test_generated_y4m_fixtures_are_deterministic_and_match_repo(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = generate(pathlib.Path(tmp))
            for key, expected_header in [
                ("main", b"YUV4MPEG2 W4 H4 F25:1 Ip A1:1 C420\n"),
                ("overlay", b"YUV4MPEG2 W2 H2 F25:1 Ip A1:1 C420\n"),
            ]:
                data = paths[key].read_bytes()
                self.assertTrue(data.startswith(expected_header))
                self.assertEqual(data.count(b"FRAME\n"), 3)
                committed = (FIXTURES / paths[key].name).read_bytes()
                self.assertEqual(data, committed)


if __name__ == "__main__":
    unittest.main()
