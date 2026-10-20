import unittest
from generate_avc_matroska_pipeline_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_saved_untimed_companions_match_authored_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
