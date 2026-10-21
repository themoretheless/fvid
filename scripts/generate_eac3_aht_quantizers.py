"""Owned AHT VQ/GAQ sample frames and analytic conventional controls.
Flat exponent-8 spectra permit closed-form masking; no decoder or process runs.
"""
import json,math
from pathlib import Path
from generate_eac3_playback import BitWriter,patch_crc
from generate_eac3_bsid import movie
from generate_eac3_aht import conventional
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/eac3-aht-quantizers'
HEBAP=list(range(8))+[8]*4+[9]*3+sum(([p]*4 for p in range(10,18)),[])+[18]*8+[19]*9
WIDTH=[0,2,3,4,5,7,8,9,3,4,5,6,7,8,9,10,11,12,14,16]
A0=[0x1249,0x0889,0x0421,0x0208,0x0102,0x0081,0x0040,0x0020,0x0010,8,2,0]
A=[[0xd555,0xedb7],[0xc925,0xe666],[0xc444,0xe319],[0xc211,0xe186],[0xc104,0xe0c2],[0xc081,0xe060],[0xc040,0xe030],[0xc020,0xe018],[0xc010,0xe00c]]
B=[[0xeaab,0xfb6e],[0xd249,0xeccd],[0xc889,0xe632],[0xc421,0xe30c],[0xc208,0xe183],[0xc102,0xe0c1],[0xc081,0xe060],[0xc040,0xe030],[0xc020,0xe018]]
def q15(x):return (x if x<32768 else x-65536)/32768

def allocation(offset,exponent=8):
 # For the exponent-8/10 spectra here, bands rise monotonically and low-compensation is zero.
 # Fixed E-AC-3 params give excitation=band PSD-640. The quiet-band knee is
 # 2304 and all hearing thresholds for these 73 bins are below that mask.
 assert exponent in (8,10)
 result=[]
 for n in [1]*28+[3]*7+[6]*4:
  power=3072-128*exponent
  psd=power+math.floor(64*math.log2(n))
  mask=psd-640+((2304-psd)>>2)
  level=((max(0,mask-offset+2048))&0x1fe0)-2048
  pointer=HEBAP[max(0,min(63,(power-level)>>5))]
  result.extend([pointer]*n)
 assert len(result)==73 and min(result)>0
 return result

def vectors(pointers,mode,seed,books):
 words=[(mode,2)];gains=[0]*73;active=[i for i,p in enumerate(pointers) if mode and 7<p<(12 if mode<2 else 17)]
 for i in active:gains[i]=((i+seed)%2)*mode if mode<3 else (i+seed)%3
 if mode==3:
  for j in range(0,len(active),3):
   g=[gains[i]for i in active[j:j+3]]+[0,0];words.append((g[0]*9+g[1]*3+g[2],5))
 else:
  for i in active:words.append((gains[i]//mode,1))
 spectra=[];seen={}
 for i,p in enumerate(pointers):
  if p==0:
   # No coded word; callers with disabled dither reconstruct true zero.
   values=[0]*6
  elif p<8:
   rows=books[str(p)];index=(i*13+seed*29)%len(rows);words.append((index,WIDTH[p]));values=rows[index];seen.setdefault(p,set()).add(index)
  else:
   values=[];m=WIDTH[p];g=gains[i];small=m-g
   for j in range(6):
    k=(i+j+seed)%6
    if g and k>=3:
     words.append((-(1<<(small-1)),small));width=m-(2-g)
     q=[0,1,-1,-(1<<(width-1)),(1<<(width-1))-1,-2][k]
     words.append((q,width));x=q/(1<<(width-1));y=x*(1+q15(A[p-8][g-1]))+(1/(1<<g)if q>=0 else q15(B[p-8][g-1]))
    else:
     # Stay away from the escape code; exercise both signs and zero.
     q=[0,(1<<(small-1))-1,-((1<<(small-1))-1)][k%3]
     words.append((q,small));y=q/(1<<(m-1))
     if not g:y*=1+A0[p-8]/32768
    values.append(y*32768)
  spectra.append(values)
 return words,spectra,seen

def header(w,converted,aht,offset):
 w.put(0x0b77,16).put(2 if converted else 0,2).put(0,3).put(2047,11)
 w.put(0,2).put(3,2).put(2,3).put(0,1).put(16,5).put(31,5).zeros(3)
 if converted:w.put(20,6)
 w.put(0,1).put(1,1).put(int(aht),1).put(0,2).zeros(8).zeros(6)
 for b in range(6):w.put(3 if b==0 else 0,2).put(3 if b==0 else 0,2)
 if not converted:w.zeros(10)
 if aht:w.put(1,1).put(1,1)
 code=offset//4+240;w.put(code//16,6).put(code%16,4).put(0,1)

def packet(converted,offset,words,values,shifts,control,exponent=8):
 w=BitWriter();header(w,converted,not control,1584 if control else offset)
 for b in range(6):
  w.zeros(2)
  if b==0:
   w.zeros(16)
   for ch in range(2):
    w.put(exponent-shifts[ch] if control else exponent,4)
    for i in range(6):w.put(62,7)
    w.put(0,2)
  else:w.put(0,1)
  if not converted:w.put(0,1)
  for ch in range(2):
   if control:
    for i in range(73):w.put(round(conventional(values[ch][i],b)/(1<<shifts[ch]))&65535,16)
   elif b==0:
    for q,width in words[ch]:w.put(q&((1<<width)-1),width)
 return patch_crc(w.to_bytes(4096))

def generate():
 assert len(HEBAP)==64
 books=json.loads(Path(__file__).with_name('eac3_aht_author_codebooks.json').read_text())['books']
 offsets=[];covered=set()
 for offset in range(-448,1585,32):
  pointers=allocation(offset)
  if set(pointers)-covered:offsets.append(offset);covered.update(pointers)
 assert covered==set(range(1,20)),covered
 files={};cases=[];words_seen={}
 for offset in offsets:
  pointers=allocation(offset)
  for mode in range(4):
   for converted,variant in [(c,v)for v in range(4 if set(pointers)&{6,7} else 1)for c in [False,True]]:
    seed=len(cases)//2+variant*71;name=f'o{offset}-g{mode}-t{2 if converted else 0}'+(f'-v{variant}'if variant else'')
    authored=[vectors(pointers,mode,seed+ch,books)for ch in range(2)]
    words=[v[0]for v in authored];values=[v[1]for v in authored]
    for _,_,seen in authored:
     for p,indices in seen.items():words_seen.setdefault(p,set()).update(indices)
    peaks=[max(abs(conventional(v,b))for v in values[ch]for b in range(6))for ch in range(2)]
    shifts=[max(0,math.ceil(math.log2(max(1,x/32760))))for x in peaks]
    assert max(shifts)<=4
    l1=max(sum(abs(conventional(v,b)-round(conventional(v,b)/(1<<shifts[ch]))*(1<<shifts[ch]))for v in values[ch])for ch in range(2)for b in range(6))
    for exponent in [8,10]:
     wire_offset=offset+(exponent-8)*32
     assert allocation(wire_offset,exponent)==pointers
     label=name+('-headroom'if exponent==10 else'')
     raw=packet(converted,wire_offset,words,values,shifts,False,exponent);control=packet(converted,wire_offset,words,values,shifts,True,exponent)
     for suffix,data in [('.ec3',raw*3),('-control.ec3',control*3),('.mkv',movie([raw]*3)),('-control.mkv',movie([control]*3))]:files[label+suffix]=data
     cases.append(dict(name=label,file=label+'.ec3',control=label+'-control.ec3',video=label+'.mkv',control_video=label+'-control.mkv',blocks=6,mask=3,seed=seed,mode=mode,pointers=pointers,quantization_bound=4*l1/(32768*(1<<exponent)),offset=wire_offset,base_offset=offset,exponent=exponent,shifts=shifts))
 assert {str(k):len(v)for k,v in words_seen.items()}=={'1':4,'2':8,'3':16,'4':32,'5':128,'6':256,'7':512}
 files['manifest.json']=(json.dumps(dict(cases=cases,covered=sorted(covered),vq_codewords={str(k):len(v)for k,v in words_seen.items()}),indent=2)+'\n').encode()
 return files
if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for n,b in generate().items():(OUT/n).write_bytes(b)
