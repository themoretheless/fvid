import unittest
from generate_matroska_default_track_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_default_candidates_and_pcm_match_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
