#!/usr/bin/env python3
"""Reproduce authored subtitle/video bytes without codecs or network."""
from pathlib import Path
import tempfile
import unittest
from generate_subtitle_native_clock_regressions import generate, ROOT
class Reproduction(unittest.TestCase):
    def test_authored_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            generate(root)
            self.assertEqual({p.name for p in root.iterdir()}, {p.name for p in ROOT.iterdir()})
            self.assertEqual(len(list(root.iterdir())), 6)
            for path in root.iterdir():
                self.assertEqual(path.read_bytes(), (ROOT/path.name).read_bytes(), path.name)
if __name__ == '__main__':
    unittest.main()
