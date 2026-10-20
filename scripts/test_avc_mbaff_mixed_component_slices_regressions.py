import json,hashlib,unittest
from generate_avc_mbaff_mixed_component_slices_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_mixed_slice_packets_and_midpoint_oracles_are_reproducible(self):
        files,cases=generate();manifest=json.loads((OUT/'manifest.json').read_text())
        self.assertEqual(cases,manifest['cases'])
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name);self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
