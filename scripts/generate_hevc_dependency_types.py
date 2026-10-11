#!/usr/bin/env python3
"""Original Annex F dependency-type tails and connected metadata prefixes."""
import json
from pathlib import Path
D=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
def ue(n):
 s=f'{n+1:b}';return '0'*(len(s)-1)+s
def packed(s):return int(s+'0'*((-len(s))%8),2).to_bytes((len(s)+7)//8,'big')
m=json.loads((D/'hevc-dpb.json').read_text())
cases=[]
config=[('sample',2,0,0,False,None),('motion',2,1,0,False,None),('both',2,2,0,False,None),('neither',2,3,0,False,None),('reserved',3,6,0,False,None),('wide',32,0xfffffffe,0,False,None),('edges',2,None,0,False,None),('external',2,1,0,False,None),('external-edges',2,None,0,False,None),('bad-alignment',2,2,0,True,'VUI alignment'),('ignored',2,2,4096,False,None),('vui',2,2,0,True,None),('bad-width',33,2,0,False,'out of range'),('bad-common',32,0xffffffff,0,False,'invalid HEVC direct'),('bad-edge',32,None,0,False,'invalid HEVC direct'),('bad-length',2,2,4097,False,'out of range')]
for name,width,common,ignored,vui,error in config:
 source=next(c for c in m if c['file']==('hevc-dpb-external.bin' if name.startswith('external') else 'hevc-dpb-explicit.bin'))
 prefix=''.join(f'{v:08b}' for v in (D/source['file']).read_bytes())[:source['end']]
 bits=prefix+ue(width-2)+str(int(common is not None));values=[]
 if common is not None:bits+=f'{common:0{width}b}'
 for i in range(3):
  value=0 if name.startswith('external') and i==0 else common if common is not None else 0xffffffff if name=='bad-edge' and i==1 else i
  if common is None and not(name.startswith('external') and i==0):bits+=f'{value:0{width}b}'
  values.append(value)
 bits+=ue(ignored)+''.join(f'{(i*37)%256:08b}' for i in range(ignored))+str(int(vui))
 if vui:
  padding=(-len(bits))%8
  if name=='bad-alignment':assert padding>0
  bits+=('0' if name=='bad-alignment' else '1')*padding
 end=len(bits)
 # Preserve uninterpreted VUI sentinel to prove that this prefix stops before it.
 if vui:bits+='10100101'
 file='hevc-dependency-types-'+name+'.bin';(D/file).write_bytes(packed(bits))
 oracle={0:(True,False),1:(False,True),2:(True,True),3:(False,False),6:(True,True),0xfffffffe:(True,True)}
 sample=[0];motion=[0]
 for value,reference in zip(values,(0,3,7)):
  s,t=oracle.get(value,(False,False));sample.append((1<<reference) if s else 0);motion.append((1<<reference) if t else 0)
 cases.append(dict(file=file,error=error,end=end,width=width,common=common,values=values,sample=sample,motion=motion,internal=source['internal'],ignored=ignored,vui=vui))
(D/'hevc-dependency-types.json').write_text(json.dumps(cases,indent=2)+'\n')
