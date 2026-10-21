"""Owned transient-window metadata witnesses; not PCM-correction acceptance."""
import json
from pathlib import Path
from generate_eac3_dependent import packet
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-transient-windows'
def generate():
 files={};cases=[]
 for name,windows in [('left',[(300,48),None]),('future',[(1023,255),(0,0)]),('right',[None,(500,1)])]:
  raw,_=packet(2,False,0,0,None,None,False,blocks_override=6,transient_windows=windows)
  files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3)
  cases.append(dict(name=name,windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]))
 control,_=packet(2,False,0,0,None,None,False,blocks_override=6);files['control.ec3']=control
 files['manifest.json']=(json.dumps(dict(cases=cases,pcm_processing_accepted=False),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
