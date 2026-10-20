import unittest
from generate_audio_stretch_clock_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_rendered_clock_witness_matches_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
