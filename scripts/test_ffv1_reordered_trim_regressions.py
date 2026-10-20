import unittest,json,hashlib
from generate_ffv1_reordered_trim_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_files_match_generator(self):
        manifest=json.loads((OUT/'manifest.json').read_text())
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
