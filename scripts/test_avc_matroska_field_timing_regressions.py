import unittest
from generate_avc_matroska_field_timing_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_saved_pair_timing_companions_match_authored_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
