import unittest
from generate_flac_block_codes_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_reproducible(self):
        for n,b in generate().items():self.assertEqual((OUT/n).read_bytes(),b,n)
if __name__=='__main__':unittest.main()
