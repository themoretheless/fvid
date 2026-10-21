"""Owned coupled AHT spectra and equivalent explicit-coordinate controls."""
import json
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_aht import spectrum,conventional
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-coupling'
def packet(mask,converted,basis,begin,phase,aht=True,refresh=False,*,zero=False,flags=None,exponent=10,dependent=False,channel_map=0x0200,gains=None):
 end=0 if begin==0 else 1
 start=37+12*begin;stop=37+12*(end+3);bands=end+3-begin
 w=BitWriter().put(0x0b77,16).put(1 if dependent else 2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).put(int(dependent),1)
 if dependent:w.put(0,8).put(1,1).put(channel_map,16)
 w.zeros(2)
 if converted:w.put(20,6)
 w.put(0,1).put(1,1).put(int(aht),1).put(0,2)
 w.zeros(2).put(int(flags is not None),1).zeros(5)
 w.put(1,1).zeros(5) # all six blocks coupled, one coupling region
 for b in range(6):
  for _ in range(3):w.put(3 if b==0 else 0,2)
 if not converted and not dependent:w.zeros(10)
 if aht:
  for bit in [2,0,1]:w.put((mask>>bit)&1,1)
 w.put(0 if zero else 63,6).put(0 if zero else 15,4).put(0,1)
 for b in range(6):
  if flags is not None:w.put(flags[b][0],1).put(flags[b][1],1)
  gain=None if gains is None else gains[b]
  w.put(int(gain is not None),1)
  if gain is not None:w.put(gain,8)
  w.put(0,1) # SPX strategy/off
  if b==0:
   w.put(0,1).put(int(phase),1).put(begin,4).put(end,4).put(1,1).zeros(bands-1)
  update=b==0 or refresh
  for ch in range(2):
   if b:w.put(int(update),1)
   if update:
    w.put(0,2)
    for _ in range(bands):w.put(3+ch,4).put(8*ch,4)
  if phase and update:
   for band in range(bands):w.put(1 if band%2==0 else 0,1)
  if b==0:
   w.zeros(2 if begin==0 else 3) # rematrix flags
   w.put(exponent//2,4) # doubled coupling absolute exponent
   for _ in range((stop-start)//12):w.put(62,7)
   for ch in range(2):
    w.put(exponent,4)
    for _ in range((start+8)//12):w.put(62,7)
    w.zeros(2)
  else:w.put(0,1) # rematrix reuse
  if not converted and not dependent:w.put(0,1)
  if b==0:w.zeros(6) # implicit first coupling leaks
  else:w.put(0,1) # coupling leaks reuse
  for plane,lo,hi in [(0,0,start),(2,start,stop),(1,0,start)]:
   active=aht and mask&(1<<plane)
   if active:
    if b:continue
    w.put(0,2)
   if zero:continue
   for bin in range(lo,hi):
    values=spectrum(bin,plane,basis)
    if active:
     for value in values:w.put(value&65535,16)
    else:w.put(round(conventional(values,b))&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for converted in [False,True]:
  for begin in [0,2]:
   start=37+12*begin;stop=73 if begin==0 else 85
   for phase in [False,True]:
    for mask in range(8):
     for basis in [None,3,'mixed']:
      name=f't{2 if converted else 0}-b{begin}-p{int(phase)}-m{mask}-{basis if basis is not None else "dc"}'
      raw=packet(mask,converted,basis,begin,phase)
      control=packet(0,converted,basis,begin,phase,False,True)
      # Conservative per-output L1 bound: each coupling coordinate gain <= 1/2.
      def error(plane,lo,hi,b):return sum(abs(conventional(spectrum(i,plane,basis),b)-round(conventional(spectrum(i,plane,basis),b)))for i in range(lo,hi))
      l1=max(error(ch,0,start,b)+.5*error(2,start,stop,b)for ch in range(2)for b in range(6))
      for suffix,data in [('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3)),('-control.mkv',movie([control]*3))]:files[name+suffix]=data
      cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,mask=mask,basis=basis,begin=begin,phase=phase,quantization_bound=4*l1/(32768*1024),exact=basis in [None,3]))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
