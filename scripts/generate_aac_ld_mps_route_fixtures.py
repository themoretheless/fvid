#!/usr/bin/env python3
"""Offline Decimal W-routing oracle with synthetic dyadic stimuli."""
import json,pathlib,struct
from decimal import Decimal as D,localcontext
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-route'
TOPOLOGY=[(1,[0,1,3,2],5,4),(1,[0,1,3,4],5,2),(3,[1,2,0],4,0),(3,[1,2,0,3,4],6,0),(3,[1,2,0,3,4],6,0),(6,[0,1],2,None),(6,[0,1],2,None),(1,[0],1,None)]
def sample(kind,t,c,q):
 if kind==0:return(D((t*11+c*7+q*3)%31-15)/16,D((t*5+c*13+q*7)%29-14)/16)
 if kind==1:return(D((t*17+c*11+q*5)%37-18)/32,D((t*13+c*3+q*17)%43-21)/32)
 return(D((t*7+c*19+q*11)%41-20)/64,D((t*3+c*17+q*13)%47-23)/64)
def main():
 facts=json.loads((ROOT/'tests/fixtures/playback-errors/aac-ld-mps-prepare/facts.json').read_text());blob=bytearray();cases=[]
 with localcontext()as ctx:
  ctx.prec=80;gain=1/D(2).sqrt()
  for tree,(direct,aux,boxes,lfe)in enumerate(TOPOLOGY):
   for bands in (4,5,7,9,12,15,23):
    for bins in (32,64,128):
     mapping=facts[f'QMF{bands}'][:bins]
     for trial in range(3):
      cutoffs=[[0,bands//2,bands][(trial+b)%3]if b!=lfe else 0 for b in range(boxes)]
      modes=[(b+trial)%6 for b in range(bands)]if tree in(2,3,4)else[]
      case=dict(tree=tree,bands=bands,bins=bins,trial=trial,direct=direct,aux=aux,cutoffs=cutoffs,modes=modes,offset=len(blob));cases.append(case)
      for wet in (False,True):
       for t in range(2):
        for c in range(direct+len(aux)):
         for q,band in enumerate(mapping):
          zero=(D(0),D(0))
          if c<direct:value=zero if wet else sample(0,t,c,q)
          else:
           box=aux[c-direct];active=band<cutoffs[box]
           value=(zero if active else sample(1,t,c,q))if wet else(sample(2,t,box,q)if active else zero)
           if wet and c==5 and modes and modes[band]==0 and not active:
            a,b,z=sample(1,t,3,q),sample(1,t,4,q),sample(1,t,5,q);value=tuple(a[i]+b[i]+gain*z[i]for i in range(2))
          blob.extend(struct.pack('<2d',*[float(x)for x in value]))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/'expected.bin').write_bytes(blob);(OUT/'manifest.json').write_text(json.dumps(dict(scope='W-routing component only; decoded residual samples are synthetic, not decoded audio or player acceptance.',cases=cases),separators=(',',':'))+'\n');print(len(cases),len(blob))
if __name__=='__main__':main()
