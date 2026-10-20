import unittest
from generate_mp4_pcm_choice_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_authored_bytes_are_reproducible(self):
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
if __name__=='__main__': unittest.main()
