"""Owned native/base-player transient witnesses; expanded output remains pending.

No encoder, subprocess or network is needed. Playback acceptance is exercised
by tests/eac3_transient_playback.rs, not inferred from generation.
"""
import json
from pathlib import Path
from generate_eac3_dependent import packet
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-transient-native'
def generate():
 files={};cases=[]
 for name,layout,lfe in [('stereo',2,False),('dual',0,False),('mono-lfe',1,True)]:
  windows=[(450,127),None] if layout in (0,2) else [(450,127)]
  args=dict(blocks_override=6,gains=[None,0,64,None,32,None],divisor=16)
  raw,_=packet(layout,lfe,0,0,'mixed',None,False,transient_windows=windows,**args)
  control,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
  files[name+'.ec3']=raw*3
  files[name+'-control.ec3']=control*3
  files[name+'.mkv']=movie([raw]*3)
  # A damaged CRC is deliberately only a CRC concealment witness, not a
  # late syntax-fault witness or transient player acceptance.
  damaged=bytearray(raw);damaged[-1]^=1
  files[name+'-crc.ec3']=bytes(damaged)+raw*2
  cases.append(dict(name=name,layout=layout,lfe=lfe))
 args=dict(blocks_override=6,divisor=16)
 unity,_=packet(2,False,0,0,'mixed',None,False,transient_windows=[(450,127),None],**args)
 control,_=packet(2,False,0,0,'mixed',None,False,**args)
 files['unity.ec3']=unity*3;files['unity.mkv']=movie([unity]*3)
 files['unity-control.ec3']=control*3;files['unity-control.mkv']=movie([control]*3)
 files['manifest.json']=(json.dumps(dict(cases=cases,native_adapter_accepted=True,base_player_integration_accepted=True,expanded_player_integration_accepted=False),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
