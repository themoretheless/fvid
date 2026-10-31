#!/usr/bin/env python3
"""Five authored VP9 keyframes with conservative sync flags and reordered PTS."""
import json,hashlib
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import mp4,atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-seek-reordered-coverage'
def main():
 seed=generate();files={n:b for n,b in seed.items() if n!='three-colors.mp4'}
 packets=[seed[f'frame-{i%3}.packet'] for i in range(5)]
 base=mp4(packets,8,'420',size=(16,16),sync_samples=[1,3,5])
 files['ordered.mp4']=base
 def walk(data):
  out=b''
  for k,b in atoms(data):
   if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=walk(b)
   elif k in [b'mvhd',b'mdhd']:b=b[:16]+word(13)+b[20:]
   elif k==b'tkhd':b=b[:20]+word(13)+b[24:]
   out+=atom(k,b)
   if k==b'stts':out+=atom(b'ctts',word(0,5,*[x for shift in [7,8,6,9,6] for x in [1,shift]]))
  return out
 files['reordered.mp4']=walk(base);OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Five authored16x16 VP9 keyframes with conservative stss1,3,5. No encoder/network/private media.','decode_pts':[7,9,8,12,10],'seek_target':10,'expected':'Any returned sync landing retains all sequential output frames at or after its returned media time','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
