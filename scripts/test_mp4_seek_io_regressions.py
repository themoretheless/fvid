import hashlib,json,unittest
from generate_mp4_seek_io_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_authored_samples_and_video_match_saved_bytes(self):
        manifest=json.loads((OUT/'manifest.json').read_text());files=generate()
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
        self.assertEqual(len({files[f'frame-{i}.yuv16'] for i in range(3)}),3)
