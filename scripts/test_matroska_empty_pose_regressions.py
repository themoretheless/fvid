import unittest
import json
import hashlib
from generate_matroska_empty_pose_regressions import OUT, generate
class Fixtures(unittest.TestCase):
    def test_generated_videos_match_checked_in_bytes(self):
        manifest=json.loads((OUT/'manifest.json').read_text())
        for name,data in generate().items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
