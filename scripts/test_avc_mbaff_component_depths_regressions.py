import hashlib,json,unittest
from generate_avc_mbaff_component_depths_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_packets_samples_and_video_are_reproducible(self):
        files,cases=generate();manifest=json.loads((OUT/'manifest.json').read_text())
        self.assertEqual(cases,manifest['cases'])
        self.assertEqual(set(files),set(manifest['files']))
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
