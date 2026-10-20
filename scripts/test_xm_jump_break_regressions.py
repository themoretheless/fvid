#!/usr/bin/env python3
import unittest
from generate_xm_jump_break_regressions import generate, OUT
class Reproduction(unittest.TestCase):
    def test_authored_bytes(self):
        files=generate()
        self.assertEqual(set(files), {p.name for p in OUT.iterdir()})
        for name,data in files.items():self.assertEqual(data,(OUT/name).read_bytes(),name)
if __name__=='__main__':unittest.main()
