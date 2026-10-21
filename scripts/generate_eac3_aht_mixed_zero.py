"""Owned mixed zero/VQ/GAQ AHT spectra with independent scalar controls.
Flat exponent-10 masking is analytic; delta segments operate on critical bands
as specified by ETSI 6.2.2.6. No production decoder, process or network is used.
"""
import json,math
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_bsid import movie
from generate_eac3_aht import conventional
from generate_eac3_aht_quantizers import HEBAP,vectors
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-mixed-zero'
BANDS=[1]*28+[3]*7+[6]*4
def deltas(start,ch):
 out=[0]*39
 for segment in range(8):
  for band in range(start+4*segment,start+4*segment+4):out[band]=(-512 if (segment+ch)%2==0 else 512)
 return out
def allocation(offset,start,ch):
 result=[];adjust=deltas(start,ch);power=1792
 for band,n in enumerate(BANDS):
  psd=power+math.floor(64*math.log2(n));mask=psd-640+((2304-psd)>>2)+adjust[band]
  level=((max(0,mask-offset+2048))&0x1fe0)-2048
  result.extend([HEBAP[max(0,min(63,(power-level)>>5))]]*n)
 assert len(result)==73 and 0 in result and max(result)>=8
 return result
def packet(converted,offset,start,words,values,shifts,control):
 w=BitWriter().put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).zeros(3)
 if converted:w.put(20,6)
 w.put(0,1).put(1,1).put(int(not control),1).put(0,2)
 w.zeros(2).put(1,1).zeros(2).put(int(not control),1).zeros(2)
 w.zeros(6)
 for b in range(6):w.put(3 if b==0 else 0,2).put(3 if b==0 else 0,2)
 if not converted:w.zeros(10)
 if not control:w.put(1,1).put(1,1)
 code=(1584 if control else offset)//4+240
 w.put(code//16,6).put(code%16,4).put(0,1)
 for b in range(6):
  w.zeros(4) # disabled dither on both channels, dynrng, SPX
  if b==0:
   w.zeros(16)
   for ch in range(2):
    w.put(10-shifts[ch]if control else 10,4)
    for _ in range(6):w.put(62,7)
    w.zeros(2)
  else:w.put(0,1)
  if not converted:w.put(0,1)
  if not control:
   w.put(int(b==0),1)
   if b==0:
    w.put(1,2).put(1,2)
    for ch in range(2):
     w.put(7,3)
     for segment in range(8):w.put(start if segment==0 else 0,5).put(4,4).put(0 if (segment+ch)%2==0 else 7,3)
  for ch in range(2):
   if control:
    for i in range(73):w.put(round(conventional(values[ch][i],b)/(1<<shifts[ch]))&65535,16)
   elif b==0:
    for q,width in words[ch]:w.put(q&((1<<width)-1),width)
 return patch_crc(w.to_bytes(4096))
def generate():
 books=json.loads(Path(__file__).with_name('eac3_aht_author_codebooks.json').read_text())['books']
 files={};cases=[];covered=set();seen={}
 for base in range(-576,-63,32):
  offset=base+64
  for start in[0,7]:
   pointers=[allocation(offset,start,ch)for ch in range(2)]
   for p in pointers:covered.update(p)
   for mode in range(4):
    for converted in[False,True]:
     seed=len(cases)*3+5;authored=[vectors(pointers[ch],mode,seed+11*ch,books)for ch in range(2)]
     words=[v[0]for v in authored];values=[v[1]for v in authored]
     for _,_,used in authored:
      for p,indices in used.items():seen.setdefault(p,set()).update(indices)
     peaks=[max(abs(conventional(v,b))for v in values[ch]for b in range(6))for ch in range(2)]
     shifts=[max(0,math.ceil(math.log2(max(1,p/32760))))for p in peaks];assert max(shifts)<=4
     l1=max(sum(abs(conventional(v,b)-round(conventional(v,b)/(1<<shifts[ch]))*(1<<shifts[ch]))for v in values[ch])for ch in range(2)for b in range(6))
     name=f'o{base}-s{start}-g{mode}-t{2 if converted else 0}'
     raw=packet(converted,offset,start,words,values,shifts,False);control=packet(converted,offset,start,words,values,shifts,True)
     for suffix,data in[('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3)),('-control.mkv',movie([control]*3))]:files[name+suffix]=data
     cases.append(dict(name=name,file=name+'.ec3',control=name+'-control.ec3',video=name+'.mkv',control_video=name+'-control.mkv',blocks=6,mask=3,base_offset=base,offset=offset,start=start,mode=mode,exponent=10,pointers=pointers,shifts=shifts,quantization_bound=4*l1/(32768*1024),exact=False))
 assert covered==set(range(15)),covered
 files['manifest.json']=(json.dumps(dict(cases=cases,covered=sorted(covered),vq_codewords={str(p):len(v)for p,v in seen.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
