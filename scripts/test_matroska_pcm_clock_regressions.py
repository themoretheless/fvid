import unittest
from generate_matroska_pcm_clock_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_single_stream_clock_and_sample_oracle_match_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
