#!/usr/bin/env python3
"""Short authored dense-index videos for decoder/reorder budget admission."""
import json,hashlib
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import mp4,atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-ordered-index-budget'
def main():
 seed=generate();files={n:b for n,b in seed.items() if n!='three-colors.mp4'}
 def movie(count,reordered=False):
  raw=mp4([seed[f'frame-{i%3}.packet'] for i in range(count)],8,'420',size=(16,16),sync_samples=list(range(1,count+1)))
  def walk(data):
   out=b''
   for k,b in atoms(data):
    if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
     b=walk(b)
     if k==b'stbl' and reordered:b+=atom(b'ctts',word(1<<24,4,1,0,1,1,1,0xffffffff,count-3,0))
    elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(1000000)+b[16:]
    out+=atom(k,b)
   return out
  return walk(raw)
 files['small-control.mp4']=movie(3);files['ordered.mp4']=movie(150000);files['reordered.mp4']=movie(150000,True)
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Three authored independent16x16 VP9 keyframes repeat; no private media/parameters,encoder or network. Dense150000-sample clips last150ms using a1MHz container clock. Reordered companion swaps only the second/third PTS via signed CTTS, all packet bytes retained.','samples':150000,'duration_ns':150000000,'tight_decoder_budget':2097152,'existing_suffix_index_payload_bytes':1200000,'scope':'Decoder/reorder working budget, not total process or demux metadata RAM.','sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
