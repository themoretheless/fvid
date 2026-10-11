#!/usr/bin/env python3
"""Original Annex F OLS metadata prefixes; not full VPS/video streams."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(n):
 s=f'{n+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
m=json.loads((D/'hevc-dependency-prefix.json').read_text());c=next(c for c in m if 'chain' in c['file'])
prefix=''.join(f'{b:08b}' for b in (D/c['file']).read_bytes())[:c['end']]+'000'
cases=[]
for name,idc,extra,last,error in [
 ('all',0,0,1,None),('highest',1,0,1,None),('explicit',2,0,1,None),
 ('reserved',3,0,1,None),('additional',1,3,1,None),('maximum',1,1023,1,None),
 ('empty',2,0,1,'empty HEVC output'),('orphan',2,0,1,'unnecessary in every'),
 ('bad-profile',1,0,2,'PTL index out of range'),('missing-profile',1,0,0,'at least two PTLs'),
 ('too-many',1,1024,1,'out of range'),
 ('mapped',1,2,1,None),('bad-set-index',1,1,1,'layer set index out of range'),
 ('extra-profile',1,0,2,None),
 ('subset',2,1,1,None),
]:
 masks=[1,9,137,32905] if name=='bad-set-index' else [1,137,32905] if name=='mapped' else [1,32905]
 bits=prefix+ue(last)
 if last==2:bits+='0'+f'{120:08b}'
 bits+=ue(extra)+f'{idc:02b}'
 outputs=[];alternatives=[];indices=[];required=[]
 for i in range(1,len(masks)+extra):
  index=i if i<len(masks) else 1 if name!='mapped' else 1+(i%2)
  if i>=len(masks) and len(masks)>2:
   width=(len(masks)-2).bit_length()
   bits+=f'{3 if name=="bad-set-index" else index-1:0{width}b}'
  partial=name=='orphan' or (name=='subset' and i==1)
  flags='0000' if name=='empty' else '0010' if partial else '0'*(bin(masks[index]).count('1')-1)+'1'
  explicit=i>=len(masks) or idc>=2
  if explicit:bits+=flags
  out=1<<(masks[index].bit_length()-1) if idc!=0 or explicit else masks[index]
  if partial:out=128
  if name=='empty':out=0
  necessary=masks[index] if out else 0
  if partial:necessary=137
  for j in range(bin(necessary).count('1')):bits+=('11' if name=='bad-profile' else '01' if last==2 else '1')
  alt=bin(out).count('1')==1
  if alt:bits+='1'
  outputs.append(out);alternatives.append(alt);indices.append(index);required.append(necessary)
 f='hevc-output-sets-'+name+'.bin';(D/f).write_bytes(packed(bits))
 cases.append(dict(file=f,error=error,end=len(bits),idc=idc,outputs=outputs,alternatives=alternatives,masks=masks,indices=indices,required=required,profiles=last+1))
(D/'hevc-output-sets.json').write_text(json.dumps(cases,indent=2)+'\n')
