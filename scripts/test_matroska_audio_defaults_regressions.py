import hashlib,json,unittest
from generate_matroska_audio_defaults_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_saved_authored_files_are_reproducible(self):
        manifest=json.loads((OUT/'manifest.json').read_text())
        files=generate()
        self.assertEqual(set(files),set(manifest['files']))
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
