"""Owned zero-bit coupling/dither witnesses; no external encoder required."""
import json
from pathlib import Path
from generate_eac3_aht_coupling import packet
from generate_eac3_zero_bit import PATTERNS
from generate_eac3_bsid import movie

OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-coupling-zero'

def generate():
 files={};cases=[]
 for exponent in [8,12]:
  for converted in [False,True]:
   for begin,phase in [(0,False),(2,True)]:
    for mask in range(8):
     for pattern,flags in PATTERNS.items():
      name=f'c-e{exponent}-t{2 if converted else 0}-b{begin}-p{int(phase)}-m{mask}-{pattern}'
      raw=packet(mask,converted,None,begin,phase,zero=True,flags=flags,exponent=exponent)
      files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3)
      cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',blocks=6,channels=2,mask=mask,exponent=exponent,allocated=False,pattern=pattern,flags=flags))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
