#!/usr/bin/env python3
"""Original Annex F DPB metadata prefixes, not complete streams."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(n):
 s=f'{n+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
def bits(c):return ''.join(f'{v:08b}' for v in (D/c['file']).read_bytes())[:c['end']]
ols=json.loads((D/'hevc-output-sets.json').read_text())
highest=next(c for c in ols if c['file']=='hevc-output-sets-highest.bin')
subset=next(c for c in ols if c['file']=='hevc-output-sets-subset.bin')
rep=next(c for c in json.loads((D/'hevc-representation.json').read_text()) if c['file']=='hevc-representation-c1-s0-d8.bin')
tail=bits(rep)[highest['end']:]
cases=[]
chain=next(c for c in json.loads((D/'hevc-dependency-prefix.json').read_text()) if c['file']=='hevc-dependency-prefix-chain.bin')
for name in ('inferred','explicit','partial','decrease','wide','subset','external'):
 source=subset if name=='subset' else highest
 prefix=bits(source)+tail
 # Additional PTL now has two temporal sublayer flags and reserved bits.
 prefix=prefix[:8]+'0'*16+prefix[8:]
 if name=='external':
  prefix=prefix[24:] # External base: no additional PTL before splitting_flag.
  insertion=chain['end']-8+3+len(ue(1))
  prefix=prefix[:insertion]+'0'+f'{120:08b}'+'0'*16+prefix[insertion:]
 syntax=prefix;expected=[]
 for out_index,necessary in enumerate(source['required']):
  flags=name!='inferred';syntax+=str(int(flags));rows=[]
  for tid in range(3):
   present=tid==0 or (flags and not(name=='partial' and tid==1))
   if tid and flags:syntax+=str(int(present))
   if present:
    capacities=[2+tid,3+tid,4+tid,5+tid]
    if name=='decrease' and tid==1:capacities[2]=1
    if name=='wide':capacities=[0xffffffff]*4
    capacities=[v if necessary&(1<<id) else None for v,id in zip(capacities,(0,3,7,15))]
    if name=='external':capacities[0]=1
    reorder=0xfffffffe if name=='wide' else tid+2
    latency=0xfffffffe if name=='wide' else 0 if tid==0 else tid+1
    syntax+=''.join(ue(v-1) for k,v in enumerate(capacities) if v is not None and not(name=='external' and k==0))+ue(reorder)+ue(latency)
   else:capacities,reorder,latency=rows[-1]['pictures'],rows[-1]['reorder'],rows[-1]['latency']
   rows.append(dict(present=present,pictures=capacities,reorder=reorder,latency=latency,max_latency=None if latency==0 else reorder+latency-1))
  expected.append(rows)
 file='hevc-dpb-'+name+'.bin';(D/file).write_bytes(packed(syntax))
 cases.append(dict(file=file,error='capacity decreases' if name=='decrease' else None,end=len(syntax),sets=expected,internal=name!='external'))
(D/'hevc-dpb.json').write_text(json.dumps(cases,indent=2)+'\n')
