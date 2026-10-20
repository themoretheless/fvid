import unittest
from generate_audio_duplicate_chunks_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_authored_bytes_are_reproducible(self):
        for n,b in generate().items():self.assertEqual((OUT/n).read_bytes(),b,n)
if __name__=='__main__':unittest.main()
