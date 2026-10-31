#!/usr/bin/env python3
"""Author packed-mdhd language boundary cases; no encoder or network."""
import hashlib,json,struct
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-packed-language'
def main():
 seed=generate();files={n:b for n,b in seed.items() if n!='three-colors.mp4'};cases=[]
 def packed(s):return sum((ord(c)-96)<<shift for c,shift in zip(s,[10,5,0]))
 values=[('eng',packed('eng'),'eng'),('rus',packed('rus'),'rus'),('und',packed('und'),''),('unspecified',0x7fff,''),('reserved',packed('eng')|0x8000,''),('zero',0,'')]
 for shift in [10,5,0]:
  for bad in [0,27,28,29,30,31]:values.append((f'group-{shift}-{bad}',(packed('eng')&~(31<<shift))|(bad<<shift),''))
 for version in [0,1]:
  for label,value,expected in values:
   def walk(data):
    out=b''
    for k,b in atoms(data):
     if k in [b'moov',b'trak',b'mdia']:b=walk(b)
     elif k==b'mdhd':
      b=b[:20]+struct.pack('>H',value)+b[22:]
      if version:b=word(1<<24)+bytes(16)+b[12:16]+struct.pack('>Q',int.from_bytes(b[16:20],'big'))+b[20:]
     out+=atom(k,b)
    return out
   name=f'v{version}-{label}.mp4';files[name]=walk(seed['three-colors.mp4']);cases.append({'file':name,'word':value,'expected':expected})
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in files.items():(OUT/n).write_bytes(b)
 (OUT/'manifest.json').write_text(json.dumps({'provenance':'Authored three-color VP9 frames; metadata-only packed ISO language boundaries; no private media','cases':cases,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
