import unittest
from generate_flac_number_timeline_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_checked_in_bytes(self):
        expected=generate()
        self.assertEqual(set(expected),{p.name for p in OUT.iterdir() if p.is_file()})
        for n,b in expected.items():self.assertEqual((OUT/n).read_bytes(),b,n)
if __name__=='__main__':unittest.main()
