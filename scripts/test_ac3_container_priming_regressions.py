"""Saved fixture integrity only; never invoke its explicit FFmpeg generator."""
import unittest,json,hashlib
from generate_ac3_container_priming_regressions import OUT
class Fixture(unittest.TestCase):
    def test_saved_authored_bytes_and_reference_hashes(self):
        m=json.loads((OUT/'manifest.json').read_text())
        for name,expected in m['sha256'].items():self.assertEqual(hashlib.sha256((OUT/name).read_bytes()).hexdigest(),expected,name)
if __name__=='__main__':unittest.main()
