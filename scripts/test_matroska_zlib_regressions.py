import json,hashlib,unittest,zlib
from generate_matroska_zlib_regressions import OUT,generate
class Fixtures(unittest.TestCase):
    def test_every_generated_video_and_payload_matches_saved_bytes(self):
        files,cases,invalid=generate();manifest=json.loads((OUT/'manifest.json').read_text())
        self.assertEqual(cases,manifest['cases']);self.assertEqual(invalid,manifest['invalid'])
        for name,data in files.items():
            self.assertEqual((OUT/name).read_bytes(),data,name)
            self.assertEqual(hashlib.sha256(data).hexdigest(),manifest['files'][name])
        self.assertEqual(zlib.decompress(files['frame.zlib']),files['frame.packet'])
        raw=files['frame.packet'];compressor=zlib.compressobj(strategy=zlib.Z_FIXED)
        fixed=compressor.compress(raw)+compressor.flush()
        for name,stream,block_type in [('plain.mkv',files['frame.zlib'],2),('fixed-deflate.mkv',fixed,1),('stored-deflate.mkv',zlib.compress(raw,0),0)]:
            self.assertEqual((stream[2]>>1)&3,block_type)
            self.assertEqual(files[name].count(stream),1)
        self.assertEqual(files['plain.mkv'],(OUT.parent/'matroska-header-stripping/unsupported-zlib.mkv').read_bytes())
