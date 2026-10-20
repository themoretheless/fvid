import unittest
from generate_midi_fine_tuning_regressions import generate,OUT
class Fixture(unittest.TestCase):
    def test_reproducible(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
if __name__=='__main__':unittest.main()
