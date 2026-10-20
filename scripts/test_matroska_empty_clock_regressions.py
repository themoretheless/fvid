import hashlib,json,unittest
from generate_matroska_empty_clock_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_clock_fixtures_match_hashes_and_generator(self):
        manifest=json.loads((OUT/'manifest.json').read_text())
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
