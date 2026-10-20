import hashlib,json,unittest
from pathlib import Path
class Fixtures(unittest.TestCase):
    def test_generated_codec_witnesses_match_recorded_hashes(self):
        base=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-audio-sample-clocks'
        manifest=json.loads((base/'manifest.json').read_text())
        self.assertEqual(len(manifest['cases']),6)
        for case in manifest['cases']:self.assertEqual(hashlib.sha256((base/case['file']).read_bytes()).hexdigest(),case['sha256'])
