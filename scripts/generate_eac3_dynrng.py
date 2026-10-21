"""Owned complete normal-dynrng code range; small signal avoids clipping."""
import json
from pathlib import Path
from generate_eac3_dependent import packet
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-dynrng'
def generate():
 files={};cases=[]
 control,_=packet(2,False,0,0,None,None,False)
 files['control.ec3']=control*3;files['control.mkv']=movie([control]*3)
 powers=[1,2,4,8,1/16,1/8,1/4,1/2]
 for code in range(256):
  raw,_=packet(2,False,0,0,None,None,False,gains=[code]+[None]*5)
  name=f'code-{code:03}';files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3)
  cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',code=code,gain=powers[code>>5]*(32+(code&31))/32))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
