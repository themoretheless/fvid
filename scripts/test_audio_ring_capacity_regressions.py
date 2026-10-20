import unittest
from generate_audio_ring_capacity_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_large_access_unit_matches_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
