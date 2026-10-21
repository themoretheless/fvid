"""Owned active AHT scalar streams and equivalent conventional scalar controls."""
import json,math
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_bsid import movie
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht'
def spectrum(bin,ch,basis=None):
 q=((bin%3)-1)*(1024 if ch==0 else -2048)
 values=[q,0,0,0,0,0]
 if basis is not None:
  for j in ([basis] if isinstance(basis,int) else range(1,6)):
   values[j]=(((bin+j+ch)%5)-2)*256
 return values
def conventional(values,block):
 return values[0]+math.sqrt(2)*sum(values[j]*math.cos((2*block+1)*j*math.pi/12) for j in range(1,6))
def packet(mask,converted,aht=True,basis=None):
 w=BitWriter();w.put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).put(0,1).put(0,1).put(0,1)
 if converted:w.put(20,6)
 w.put(0,1) # addbsi
 w.put(1,1).put(int(aht),1).put(0,2) # per-block exponents, ahte, frame SNR
 w.zeros(8) # transproc/blksw/dither/bamode/fgain/dba/skip/spxatten
 w.zeros(6) # coupling off
 for b in range(6):w.put(3 if b==0 else 0,2).put(3 if b==0 else 0,2)
 if not converted:w.zeros(10)
 if aht:w.put(mask&1,1).put((mask>>1)&1,1)
 w.put(63,6).put(15,4).put(0,1) # high allocation, no block starts
 for b in range(6):
  w.put(0,1).put(0,1) # dynrng and spx strategy/off
  if b==0:
   w.zeros(4).zeros(12) # rematrix off and two bandwidth codes
   for ch in range(2):
    w.put(8,4)
    for i in range(6):w.put(62,7)
    w.put(0,2)
  else:w.put(0,1) # rematrix strategy reused
  if not converted:w.put(0,1)
  for ch in range(2):
   active=aht and mask&(1<<ch)
   if active:
    if b!=0:continue
    w.put(0,2) # GAQ mode zero
   for bin in range(73):
    values=spectrum(bin,ch,basis)
    if active:
     for value in values:w.put(value&65535,16)
    else:w.put(round(conventional(values,b))&65535,16)
 return patch_crc(w.to_bytes(4096))
def generate():
 files={};cases=[]
 for converted in [False,True]:
  for mask in range(4):
   name=f'type{2 if converted else 0}-mask{mask}';raw=packet(mask,converted);control=packet(0,converted,False)
   files[name+'.ec3']=raw*3;files[name+'-control.ec3']=control*3
   files[name+'.mkv']=movie([raw]*3);files[name+'-control.mkv']=movie([control]*3)
   cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,mask=mask))
 files['manifest.json']=(json.dumps(dict(cases=cases),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for name,data in generate().items():(OUT/name).write_bytes(data)
