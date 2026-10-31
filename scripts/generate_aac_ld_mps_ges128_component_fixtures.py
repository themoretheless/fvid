#!/usr/bin/env python3
"""Own128-band complex GES component stimuli; offline explicit generation."""
import json,math,struct
from pathlib import Path
from generate_aac_ld_mps_ges_fixtures import COUNTS
ROOT=Path(__file__).resolve().parents[1];OUT=ROOT/'tests/fixtures/playback-errors/aac-eld-mps128-ges'
def main():
 OUT.mkdir(exist_ok=True)
 samples=bytearray();components=[];cache={}
 for tree in range(8):
  inputs=[1,1,2,2,2,6,6,1][tree];outputs=[6,6,6,8,8,8,8,2][tree]
  for bands in (128,):
   for mode in ('dense','silent','cancel','high-only','low-only','upper-only'):
    blocks=[]
    for frame,slots in enumerate((1,3,8,7,4)):
     key=(inputs,outputs,bands,mode,frame);at=cache.get(key,len(samples));fresh=key not in cache;cache[key]=at;signals=[]
     for kind,chans in enumerate((inputs,outputs,outputs)):
      values=[]
      for t in range(slots):
       for ch in range(chans):
        for b in range(bands):
         active=mode!='silent'and(mode!='high-only'or b>=35)and(mode!='low-only'or b<6)and(mode!='upper-only'or b>=64)
         amplitude=0.007*(1+(frame+ch)%3);phase=(frame*83+t*13+ch*7+b*3)*0.17
         z=complex(math.sin(phase),math.cos(phase*1.13))*amplitude if active else 0j
         if kind==2:z*=(-1 if mode=='cancel' else 0.4)
         values.append(z)
         if fresh:samples.extend(struct.pack('<ff',z.real,z.imag))
      signals.append(values)
     target=None if frame in(0,2)else[[2**(((ch+frame+t//3)%5)/2-1)for t in range(slots)]if(ch+frame)%3!=1 else None for ch in range(COUNTS[tree])]
     blocks.append(dict(offset=at,slots=slots,targets=target))
    components.append(dict(tree=tree,bands=bands,inputs=inputs,outputs=outputs,mode=mode,blocks=blocks))
 (OUT/'component-input.f32le').write_bytes(samples);(OUT/'component.json').write_text(json.dumps(dict(scope='Own complex QMF signals; independent GES reference is generated separately. Disabled blocks still warm envelope state.',cases=components),separators=(',',':'))+'\n')
 print(len(components),'128-band GES component programmes')
if __name__=='__main__':main()
