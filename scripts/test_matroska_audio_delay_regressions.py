import unittest
from generate_matroska_audio_delay_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_codec_delay_windows_match_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
