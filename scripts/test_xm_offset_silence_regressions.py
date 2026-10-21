#!/usr/bin/env python3
import unittest
from generate_xm_offset_silence_regressions import OUT, generate

class Fixtures(unittest.TestCase):
    def test_checked_in_bytes(self):
        expected = generate()
        self.assertEqual(set(expected),{p.name for p in OUT.iterdir() if p.is_file()})
        for name,data in expected.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)

if __name__ == '__main__':
    unittest.main()
