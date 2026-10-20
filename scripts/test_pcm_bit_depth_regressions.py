import unittest
from generate_pcm_bit_depth_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_authored_bytes_are_reproducible(self):
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
if __name__=='__main__': unittest.main()
