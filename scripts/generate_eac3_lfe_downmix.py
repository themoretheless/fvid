"""Owned LFE downmix code witnesses, no external codec or network."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-lfe-downmix'
def generate():
 files={};cases=[]
 for layout in range(8):
  channels=COUNTS[layout]+1
  args=dict(blocks_override=3,divisor=16)
  control,_=packet(layout,True,0,0,'mixed',None,False,**args)
  files[f'l{layout}-control.ec3']=control*3
  files[f'l{layout}-control.mkv']=movie([control]*3,blocks=3,channels=channels)
  for code in [None]+list(range(32)):
   raw,_=packet(layout,True,0,0,'mixed',None,False,lfe_mix_code=code,mixing_metadata=True,**args)
   name=f'l{layout}-c{code}'
   files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3,blocks=3,channels=channels)
   cases.append(dict(name=name,layout=layout,channels=channels,code=code))
 for layout in range(8):
  channels=COUNTS[layout]+1;name=f'l{layout}-changing'
  packets=[files[f'l{layout}-c{code}.ec3'][:4096]for code in [0,31,None]]
  files[name+'.ec3']=b''.join(packets)
  files[name+'.mkv']=movie(packets,blocks=3,channels=channels)
  cases.append(dict(name=name,layout=layout,channels=channels,codes=[0,31,None]))
 files['manifest.json']=(json.dumps(dict(cases=cases,lfe_downmix_accepted=True),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
