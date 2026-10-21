"""Owned short SPX-noise rewind/seek regressions, no external authoring tools."""
import json
from pathlib import Path
from generate_eac3_playback import build_spx_atten_frame
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-spx-noise'
def generate():
 files={};cases=[]
 for label,atten in [('none',None),('notch',[5,12]),('edge',[15,31])]:
  for blend in [31,0,8,16,24]:
   name=f'{label}-blend{blend}';raw=build_spx_atten_frame(atten,blend=blend)
   files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3)
   cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',blocks=6,channels=2,blend=blend,attenuation=atten,noise=blend<31))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
