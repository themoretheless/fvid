import unittest
from generate_audio_eof_drain_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_device_drain_witness_matches_saved_bytes(self):
        for name,data in generate().items():self.assertEqual((OUT/name).read_bytes(),data,name)
