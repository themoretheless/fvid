"""Owned enhanced-coupling acceptance witnesses with uncoupled scalar controls."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-enhanced-coupling'
SUB=[13,19,25,31,37]+list(range(49,254,12))
DEFAULT=[0]*9+[1,0,1,0,1,0,1,1,1,0,1,1,1]
def amplitude(code):
 if code==31:return 0.
 if code==0:return 1.
 return [27,23,19,16][(code-1)%4]/(32*(2**((code-1)//4)))
def geometry(begin):
 lo=begin*2 if begin<3 else begin+2 if begin<13 else begin*2-10
 hi=max(10,lo+1)
 return lo,hi,SUB[lo],SUB[hi]
def packet(converted,begin,seed,merged,aht,basis,control=False,*,zero=False,flags=None,exponent=10,unity=False,dependent=False,channel_map=0x0200,gains=None):
 lo,hi,start,stop=geometry(begin)
 bandmap={};band=-1
 for sb in range(lo,hi):
  if sb==lo or not(merged and DEFAULT[sb]):band+=1
  for i in range(SUB[sb],SUB[sb+1]):bandmap[i]=band
 bands=band+1
 def code(ch,bnd):return 0 if unity else (seed+ch*7+bnd*3)%32
 def value(plane,i,b):return conventional(spectrum(i,plane,basis),b)
 w=BitWriter().put(0x0b77,16).put(1 if dependent else 2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).put(int(dependent),1)
 if dependent:w.put(0,8).put(1,1).put(channel_map,16)
 w.zeros(2)
 if converted:w.put(20,6)
 w.put(0,1).put(1,1).put(int(aht and not control),1).put(0,2)
 w.zeros(2).put(int(flags is not None),1).zeros(5)
 w.put(int(not control),1).zeros(5)
 for b in range(6):
  for _ in range(2 if control else 3):w.put(1 if b==0 else 0,2)
 if not converted and not dependent:w.zeros(10)
 if aht and not control:w.put(1,1).zeros(2)
 w.put(0 if zero else 63,6).put(0 if zero else 15,4).put(0,1)
 for b in range(6):
  if flags is not None:w.put(flags[b][0],1).put(flags[b][1],1)
  gain=None if gains is None else gains[b]
  w.put(int(gain is not None),1)
  if gain is not None:w.put(gain,8)
  w.put(0,1)
  if not control:
   if b==0:
    w.put(1,1).put(begin,4).put(hi-7,4).put(int(not merged),1)
    if not merged:w.zeros(max(0,hi-max(9,lo+1)))
   w.put(0,1) # reserved before enhanced coordinates
   for ch in range(2):
    if b:
     w.put(0,1)
     if ch:w.put(0,1)
    else:
     for bnd in range(bands):w.put(code(ch,bnd),5)
     if ch:w.zeros(9*(bands-1))
    if ch:w.put(0,1)
  if b==0:
   w.zeros(4 if control else (0 if begin==0 else 1 if begin==1 else 2 if begin==2 else 3 if begin<5 else 4))
   if control:
    w.put((stop-73)//3,6).put((stop-73)//3,6)
   else:
    w.put(exponent//2,4)
    for _ in range((stop-start)//3):w.put(62,7)
   for ch in range(2):
    w.put(exponent,4)
    for _ in range(((stop if control else start)-1)//3):w.put(62,7)
    w.zeros(2)
  else:w.put(0,1)
  if not converted and not dependent:w.put(0,1)
  if not control:
   if b==0:w.zeros(6)
   else:w.put(0,1)
  for plane in ([0,1]if control else[0,2,1]):
   if plane==2:
    if aht:
     if b:continue
     w.put(0,2)
    if zero:continue
    for i in range(start,stop):
     if aht:
      for q in spectrum(i,2,basis):w.put(q&65535,16)
     else:w.put(round(value(2,i,b))&65535,16)
   else:
    if zero:continue
    for i in range(stop if control else start):
     v=value(plane,i,b)if i<start else value(2,i,b)*amplitude(code(plane,bandmap[i]))
     w.put(round(v)&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for converted in [False,True]:
  for begin in [0,1,2,3,4,5,6,8,11]:
   for seed in [0,7,17,31]:
    for merged in [False,True]:
     for aht,basis in [(False,None),(True,3),(True,'mixed')]:
      name=f't{2 if converted else 0}-b{begin}-s{seed}-g{int(merged)}-a{int(aht)}-{basis}'
      raw=packet(converted,begin,seed,merged,aht,basis);control=packet(converted,begin,seed,merged,aht,basis,True)
      # Two independently rounded scalar representations: <= 1 LSB per
      # coefficient, two overlap halves and transform factor 2.
      _,_,start,stop=geometry(begin);bound=4*(stop-start)/(32768*1024)
      for suffix,data in [('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3)),('-control.mkv',movie([control]*3))]:files[name+suffix]=data
      cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,begin=begin,seed=seed,merged=merged,aht=aht,basis=basis,quantization_bound=bound,exact=False))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
