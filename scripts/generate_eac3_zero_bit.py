"""Owned zero-bit dither flag regressions and coded-AHT positive controls."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_bsid import movie
from generate_eac3_aht import spectrum
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-zero-bit'
PATTERNS={'off':[(0,0)]*6,'on':[(1,1)]*6,'left':[(1,0)]*6,'right':[(0,1)]*6,'late':[(0,0)]*3+[(1,1)]*3,'early':[(1,1)]*3+[(0,0)]*3,'alternating':[(i%2,1-i%2)for i in range(6)]}
def packet(converted,mask,exponent,flags,allocated,*,stream_type=None,substream_id=0,channel_map=None):
 kind=(2 if converted else 0) if stream_type is None else stream_type
 w=BitWriter().put(0x0b77,16).put(kind,2).put(substream_id,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).put(int(kind==1),1)
 if kind==1:
  w.put(0,8).put(int(channel_map is not None),1)
  if channel_map is not None:w.put(channel_map,16)
 w.zeros(2)
 if kind==2:w.put(20,6)
 w.put(0,1).put(1,1).put(int(mask!=0),1).put(0,2)
 w.put(0,1).put(0,1).put(1,1).zeros(5) # explicit dither flags
 w.zeros(6)
 for b in range(6):w.put(3 if b==0 else 0,2).put(3 if b==0 else 0,2)
 if kind==0:w.zeros(10)
 if mask:w.put(mask&1,1).put((mask>>1)&1,1)
 w.put(63 if allocated else 0,6).put(15 if allocated else 0,4).put(0,1)
 for b in range(6):
  w.put(flags[b][0],1).put(flags[b][1],1).zeros(2)
  if b==0:
   w.zeros(16)
   for ch in range(2):
    w.put(exponent,4)
    for _ in range(6):w.put(62,7)
    w.zeros(2)
  else:w.put(0,1)
  if kind==0:w.put(0,1)
  for ch in range(2):
   if mask&(1<<ch):
    if b:continue
    w.put(0,2)
    if allocated:
     for i in range(73):
      for q in spectrum(i,ch):w.put(q&65535,16)
   elif allocated:
    for i in range(73):w.put(spectrum(i,ch)[0]&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for allocated in[False,True]:
  for exponent in([8]if allocated else[8,12]):
   for converted in[False,True]:
    for mask in range(4):
     for pattern in(['off','on']if allocated else PATTERNS):
      name=f'a{int(allocated)}-e{exponent}-t{2 if converted else 0}-m{mask}-{pattern}'
      raw=packet(converted,mask,exponent,PATTERNS[pattern],allocated)
      files[name+'.ec3']=raw*3;files[name+'.mkv']=movie([raw]*3)
      cases.append(dict(name=name,file=name+'.ec3',video=name+'.mkv',blocks=6,channels=2,mask=mask,exponent=exponent,allocated=allocated,pattern=pattern,flags=PATTERNS[pattern]))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
