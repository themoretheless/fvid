#!/usr/bin/env python3
"""Original Annex F prefix continuations; metadata-only, no coded pictures."""
import json
from pathlib import Path
def packed(bits):
    return int(bits+'0'*((-len(bits))%8),2).to_bytes((len(bits)+7)//8,'big')
DEST=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(v):
 s=f'{v+1:b}';return '0'*(len(s)-1)+s
m=json.loads((DEST/'hevc-dependency-prefix.json').read_text())
cases=[]
for name,counts,error,available in [
 ('none',[],None,True),('prefixes',[1,2,1],None,True),
 ('maximum',[2]*1023,None,True),('external',[2],None,False),
 ('empty',[0],'empty additional',True),('too-high',[3],'highest tree',True),
 ('missing-base',[],'unavailable HEVC base',False),
 ('too-many',[2]*1024,'out of range',True),
 ('bad-temporal',[1],'temporal sublayers',True),
 ('temporal',[2],None,True),('inferred',[1],None,True),
]:
 source=next(c for c in m if 'forest' in c['file'])
 raw=(DEST/source['file']).read_bytes();prefix=''.join(f'{b:08b}' for b in raw)[:source['end']]
 sublayers=2 if name in ('temporal','inferred') else 0
 if sublayers:prefix=prefix[:8]+'0'*16+prefix[8:]
 bits=prefix+ue(len(counts))+''.join(f'{n:02b}' for n in counts)
 # All four layers explicitly have zero temporal sublayers.
 if name=='inferred':bits+='0'
 elif name=='temporal':bits+='1'+'000'+'001'+'010'+'001'
 else:bits+='1'+('001' if name=='bad-temporal' else '000')+'000'*3
 # Edge order is reference-major: 0->2, 1->2, 1->3.
 bits+=('0' if name=='inferred' else '1'+'000'+'111'+'011')+'1'
 file='hevc-additional-sets-'+name+'.bin';(DEST/file).write_bytes(packed(bits))
 cases.append(dict(file=file,counts=counts,error=error,available=available,end=len(bits),sublayers=sublayers,temporal=([2]*4 if name=='inferred' else [0,1,2,1] if name=='temporal' else [0]*4),inferred=name=='inferred'))
(DEST/'hevc-additional-sets.json').write_text(json.dumps(cases,indent=2)+'\n')
