"""Owned heavy-compression fields, frame fallback, transient/clipping witnesses."""
import json
from pathlib import Path
from generate_eac3_dependent import packet,COUNTS,SLOTS
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-rf-compression'
def generate():
 files={};cases=[]
 def add(layout,lfe,code,transient=False,clip=False):
  name=f'l{layout}-{int(lfe)}-c{code}'+('-transient'if transient else'')+('-clip'if clip else'');nf=COUNTS[layout];channels=nf+int(lfe);divisor=1 if clip else 16
  control_name=f'l{layout}-{int(lfe)}-d{divisor}-control.mkv'
  args=dict(blocks_override=3,divisor=divisor)
  if control_name not in files:
   control,_=packet(layout,lfe,0,0,'mixed',None,False,**args)
   files[control_name]=movie([control]*3,blocks=3,channels=channels)
  words=[code,None,code^128];words2=[code^255,16,None]if layout==0 else[None]*3
  windows=[None]*nf
  if transient:
   windows[0]=(0,255)
   if nf>1:windows[-1]=(240,127)
  packets=[]
  for a,b in zip(words,words2):
   raw,_=packet(layout,lfe,0,0,'mixed',None,False,gains=[64,None,224],gains_second=[192,32,None]if layout==0 else None,compression=a,compression_second=b,transient_windows=windows if transient else None,exponent=0 if clip else 10,**args)
   packets.append(raw)
  files[name+'.ec3']=b''.join(packets);files[name+'.mkv']=movie(packets,blocks=3,channels=channels)
  slots=SLOTS[layout]+([3]if lfe else[])
  cases.append(dict(name=name,layout=layout,lfe=lfe,channels=channels,control=control_name,words=words,words2=words2,scale=1024 if clip else 1,clip=clip,transient=transient,coded_to_native=[sorted(slots).index(s)for s in slots],windows=[None if w is None else dict(location=w[0]*4,length=w[1])for w in windows]+([None]if lfe else[])))
 for layout in range(8):
  for lfe in [False,True]:
   for code in (range(256)if layout in [0,2]and not lfe else[0,15,16,127,128,143,240,255]):add(layout,lfe,code)
 for layout in [0,1,2,7]:
  for clip in [False,True]:add(layout,True,127,True,clip)
 files['manifest.json']=(json.dumps(dict(accepted=True,cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
