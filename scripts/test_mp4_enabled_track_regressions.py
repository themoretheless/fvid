import unittest
from generate_mp4_enabled_track_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_saved_movie_track_flags_and_pcm_match_authored_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
