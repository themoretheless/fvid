#!/usr/bin/env python3
"""Small authored VP9 container separating inspect and identity-copy lifetimes."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import ebml
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/matroska-copy-phase-budget'
def main():
 seed=generate();track=ebml('d7',b'\1')+ebml('83',b'\1')+ebml('86',b'V_VP9')+ebml('23e383',(20000000).to_bytes(4,'big'))+ebml('e0',ebml('b0',b'\x10')+ebml('ba',b'\x10'))+ebml('22b59c',b'eng')+ebml('22b59d',b'fr-CA')
 info=ebml('1549a966',ebml('2ad7b1',(1000000).to_bytes(3,'big'))+ebml('4489',struct.pack('>d',60.0)))
 cluster=ebml('e7',b'\0')+b''.join(ebml('a3',b'\x81'+struct.pack('>h',i*20)+b'\x80'+seed[f'frame-{i}.packet']) for i in range(3))
 data=ebml('1a45dfa3',ebml('4282',b'matroska')+ebml('4287',b'\4')+ebml('4285',b'\2'))+ebml('18538067',info+ebml('1654ae6b',ebml('ae',track))+ebml('1f43b675',cluster))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'three-colors.mkv').write_bytes(data)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Author-written standard Matroska track/header and three analytic VP9 color frames. No private media/parameters, encoder or network.','purpose':'Uncounted FVID-0337 repair: inspect reader and64KiB raw copy buffer do not overlap.70KiB is enough for each phase;64KiB and200-byte controls still refuse.','sha256':hashlib.sha256(data).hexdigest()},indent=2)+'\n')
if __name__=='__main__':main()
