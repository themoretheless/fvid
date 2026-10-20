import unittest
from generate_avc_field_inter_component_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_inter_fields_and_independent_oracles_match_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
