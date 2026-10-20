#!/usr/bin/env python3
"""Independent inverse-packing Layer III scalefactor field vectors; offline."""
import itertools,struct,json,hashlib
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp3-scalefactors';D.mkdir(exist_ok=True)
counts=[[[6,5,5,5],[9,9,9,9],[6,9,9,9]],[[6,5,7,3],[9,9,12,6],[6,9,12,6]],[[11,10,0,0],[18,18,0,0],[15,18,0,0]],[[7,7,7,0],[12,12,12,0],[6,15,12,0]],[[6,6,6,3],[12,9,9,6],[6,12,9,6]],[[8,8,5,0],[15,12,9,0],[6,18,9,0]]]
# Build inverse tables by enumerating independent encoder parameter tuples.
regular={};intensity={}
for a,b,c,d in itertools.product(range(5),range(5),range(4),range(4)):regular[((a*5+b)*4+c)*4+d]=(0,[a,b,c,d],False)
for a,b,c in itertools.product(range(5),range(5),range(4)):regular[400+(a*5+b)*4+c]=(1,[a,b,c,0],False)
for a,b in itertools.product(range(4),range(3)):regular[500+a*3+b]=(2,[a,b,0,0],True)
for a,b,c in itertools.product(range(5),range(6),range(6)):
 for scale in [0,1]:intensity[2*((a*6+b)*6+c)+scale]=(3,[a,b,c,0],False)
for a,b,c in itertools.product(range(4),range(4),range(4)):
 for scale in [0,1]:intensity[2*(180+(a*4+b)*4+c)+scale]=(4,[a,b,c,0],False)
for a,b in itertools.product(range(4),range(3)):
 for scale in [0,1]:intensity[2*(244+a*3+b)+scale]=(5,[a,b,0,0],False)
assert set(regular)==set(intensity)==set(range(512))
lengths1=[(0,0),(0,1),(0,2),(0,3),(3,0),(1,1),(1,2),(1,3),(2,1),(2,2),(2,3),(3,1),(3,2),(3,3),(4,2),(4,3)]
records=[]
layouts=[(0,0),(1,0),(1,1),(2,0),(2,1),(3,0),(3,1)]
def emit(version,kind,mixed,compress,granule,mask,is_channel,pre_input):
 values=[0]*61;widths=[0]*61;word=[]
 def put(target,width):
  value=0 if not width else (1<<width)-1 if target%4==0 else 0 if target%4==1 else (target*7+compress*3+mask*11)% (1<<width)
  values[target]=value;widths[target]=width
  word.extend((value>>i)&1 for i in range(width-1,-1,-1))
 if version==3:
  pre=pre_input;scale=False;a,b=lengths1[compress]
  if kind==2:
   if mixed:
    for i in range(8):put(i,a)
   for band in range(3 if mixed else 0,12):
    for w in range(3):put(22+3*band+w,a if band<6 else b)
  else:
   for group,(lo,hi)in enumerate([(0,6),(6,11),(11,16),(16,21)]):
    for i in range(lo,hi):
     if granule==1 and mask&(1<<group):widths[i]=4 if i<11 else 3;values[i]=(i*7+3)%(1<<widths[i])
     else:put(i,a if i<11 else b)
 else:
  table=(intensity if is_channel else regular)[compress];set_id,lengths,pre=table;scale=is_channel and compress%2==1
  layout=0 if kind!=2 else 2 if mixed else 1
  targets=list(range(21))if layout==0 else list(range(22,58))if layout==1 else list(range(6))+list(range(31,58))
  at=0
  for part,count in enumerate(counts[set_id][layout]):
   for _ in range(count):put(targets[at],lengths[part]);at+=1
  assert at==len(targets)
 prefix=len(records)%8;packed=[i%2 for i in range(prefix)]+word+[1,1,1,0,0,1,0,1];packed+=[0]*(-len(packed)%8)
 payload=bytes(sum(packed[i+j]<<(7-j)for j in range(8))for i in range(0,len(packed),8))
 header=bytes([version,kind,mixed])+struct.pack('>H',compress)+bytes([granule,mask,int(is_channel),prefix,int(pre_input)])+struct.pack('>HH',len(word),len(payload))
 expected=bytes([int(pre),int(scale),kind]+values+widths)
 records.append(header+expected+payload)
for compress in range(16):
 for kind,mixed in layouts:
  for granule in [0,1]:
   for mask in ([0]if kind==2 else range(16)):emit(3,kind,mixed,compress,granule,mask,False,(compress+mask+granule)%2)
for version in [2,0]:
 for compress in range(512):
  for kind,mixed in layouts:
   for is_channel in [False,True]:emit(version,kind,mixed,compress,0,0,is_channel,False)
output=b'FVMP3F1\0'+struct.pack('>I',len(records))+b''.join(records)
(D/'fields.bin').write_bytes(output);(D/'manifest.json').write_text(json.dumps(dict(records=len(records),sha256=hashlib.sha256(output).hexdigest(),scope='Scalefactor fields and consumed bits, not compressed PCM acceptance'),indent=2)+'\n')
print(len(records),'MP3 scalefactor field vectors')
