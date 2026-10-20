import unittest
from generate_matroska_enabled_track_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_track_flags_and_pcm_are_reproducible(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
