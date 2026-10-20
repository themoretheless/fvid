import unittest
from generate_xm_ping_pong_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_reproducible(self):
        for n,b in generate().items():self.assertEqual((OUT/n).read_bytes(),b,n)
if __name__=='__main__':unittest.main()
