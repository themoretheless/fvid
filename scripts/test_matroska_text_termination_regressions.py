import unittest,json,hashlib
from generate_matroska_text_termination_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_generated_text_videos_match_saved_bytes(self):
        manifest=json.loads((OUT/'manifest.json').read_text())
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
