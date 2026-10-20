import json,hashlib,unittest
from generate_avc_mbaff_pcm_memory_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_size_series_and_independent_samples_are_reproducible(self):
        files,cases=generate();manifest=json.loads((OUT/'manifest.json').read_text())
        self.assertEqual(cases,manifest['cases']);self.assertEqual(set(files),set(manifest['files']))
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
