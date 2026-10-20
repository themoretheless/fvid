import unittest,json,hashlib
from generate_synth_seek_control_regressions import ROOT,OUT,generate
class Fixture(unittest.TestCase):
    def test_reproducible(self):
        for n,b in generate().items():self.assertEqual((OUT/n).read_bytes(),b,n)
        for c in json.loads((OUT/'manifest.json').read_text())['cases']:self.assertEqual(hashlib.sha256((ROOT/c['source']).read_bytes()).hexdigest(),c['sha256'])
if __name__=='__main__':unittest.main()
