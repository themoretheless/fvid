"""Owned stereo+LFE zero allocation and coded-signal controls."""
import json
from pathlib import Path
from generate_eac3_aht_lfe import packet,audiovisual
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-lfe-zero'

def generate():
 files={};cases=[]
 for allocated in [False,True]:
  for exponent in ([10] if allocated else [8,12]):
   for converted in [False,True]:
    for mask in range(8):
     for enabled in [False,True]:
      name=f'a{int(allocated)}-e{exponent}-t{2 if converted else 0}-m{mask}-d{int(enabled)}'
      flags=[(int(enabled),int(enabled))]*6
      raw=packet(mask,converted,None,zero=not allocated,flags=flags,exponent=exponent)
      files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3,channels=3);files[name+'.mp4']=audiovisual([raw]*3)
      cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',mp4=name+'.mp4',allocated=allocated,exponent=exponent,mask=mask,enabled=enabled))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
