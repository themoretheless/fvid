import unittest
from generate_audio_stretch_eof_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_finite_stretch_witness_matches_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
