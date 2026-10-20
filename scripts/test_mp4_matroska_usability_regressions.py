import unittest
from generate_mp4_matroska_usability_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_saved_dual_avc_tracks_match_authored_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
