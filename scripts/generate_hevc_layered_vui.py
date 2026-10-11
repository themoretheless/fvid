#!/usr/bin/env python3
"""Original general multilayer VPS VUI prefixes; BSP HRD remains a continuation."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(n):
 s=f'{n+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
m=json.loads((D/'hevc-dependency-types.json').read_text());cases=[]
for name in ('basic','rich','external','inferred','bad-rate','bad-signal','bad-skip','bsp'):
 internal=name!='external';first=0 if internal else 1
 source=next(c for c in m if c['file']==('hevc-dependency-types-vui.bin' if internal else 'hevc-dependency-types-external.bin'))
 prefix=''.join(f'{v:08b}' for v in (D/source['file']).read_bytes())[:source['end']]
 if not internal:prefix=prefix[:-1]+'1';prefix+='1'*((-len(prefix))%8)
 rich=name not in ('basic','inferred');error=None
 bits=prefix+('011' if rich else '10')+('11' if rich else '00')
 rates=[None,None]
 if rich:
  for set_index in range(first,2):
   rows=[]
   for tid in range(3):
    rate=(set_index+tid)%2==0;pic=tid!=1
    bits+=str(int(rate))+str(int(pic))
    br=[100+set_index*10+tid,200+tid] if rate else None
    pr=[3 if name=='bad-rate' else tid%3,7680+tid] if pic else None
    if br:bits+=''.join(f'{v:016b}' for v in br)
    if pr:bits+=f'{pr[0]:02b}{pr[1]:016b}'
    rows.append(dict(bit_rate=br,picture_rate=pr))
   rates[set_index]=rows
 if name=='bad-rate':error='constant picture rate'
 indexed=name!='inferred';last=2 if rich else 0 if indexed else 3
 bits+=str(int(indexed))+(f'{last:04b}' if indexed else '')
 signals=[]
 for i in range(last+1):
  signals.append(dict(format=i%6,full_range=i%2==1,primaries=1+i,transfer=2+i,matrix=3+i))
  bits+=f'{i%6:03b}'+str(i%2)+f'{1+i:08b}{2+i:08b}{3+i:08b}'
 indices=[None]*4
 for i in range(first,4):
  value=[0,2,1,2][i] if rich else i if not indexed else 0
  if name=='bad-signal' and i==2:value=3;error='video signal index'
  indices[i]=value
  if indexed and last>0:bits+=f'{value:04b}'
 tiles=[False]*4;filters=[False]*4;alignment=[0]*4
 bits+=str(int(not rich))
 if rich:
  for i in range(first,4):
   tiles[i]=True;filters[i]=i%2==0;bits+='1'+str(int(filters[i]))
  for i in range(first+1,4):
   flag=i!=2;bits+=str(int(flag));alignment[i]=(1<<(0,3,7)[i-1]) if flag else 0
 wpp=[False]*4;bits+=str(int(not rich))
 if rich:
  for i in range(first,4):wpp[i]=i%2==0;bits+=str(int(wpp[i]))
 single=rich and name!='bad-skip';skip=rich
 if name=='bad-skip':error='higher-layer skip'
 bits+=str(int(single))+str(int(skip))+str(int(rich))
 restrictions=[]
 if rich:
  for i in range(first+1,4):
   spatial=[0,2,3][i-1];ctu=i==3;horizontal=2 if ctu else 0
   bits+=ue(spatial)
   if spatial:bits+=str(int(ctu))+(ue(horizontal) if ctu else '')
   restrictions.append(dict(dependent=i,reference=i-1,spatial=spatial,ctu=ctu,horizontal=horizontal))
 bsp=name=='bsp';bits+=str(int(bsp));end=len(bits)
 if bsp:bits+='10100101' # Deliberately uninterpreted BSP continuation sentinel.
 file='hevc-layered-vui-'+name+'.bin';(D/file).write_bytes(packed(bits))
 cases.append(dict(file=file,error=error,end=end,internal=internal,rich=rich,rates=rates,signals=signals,indices=indices,tiles=tiles,filters=filters,alignment=alignment,wpp=wpp,restrictions=restrictions,bsp=bsp))
(D/'hevc-layered-vui.json').write_text(json.dumps(cases,indent=2)+'\n')
