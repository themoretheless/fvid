import unittest
from generate_mp4_audio_clock_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_media_clocks_match_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
