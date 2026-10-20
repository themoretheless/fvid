import unittest,hashlib,json
from generate_xm_published_seek_regressions import OUT,generate
class Fixture(unittest.TestCase):
    def test_reproducible(self):
        for n,b in generate().items():self.assertEqual((OUT/n).read_bytes(),b,n)
        for c in json.loads((OUT/'manifest.json').read_text())['cases']:self.assertEqual(hashlib.sha256((OUT/c['source']).read_bytes()).hexdigest(),c['sha256'])
if __name__=='__main__':unittest.main()
