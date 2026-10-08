#!/usr/bin/env python3
"""Original 80-digit Decimal gain/noise/sine and limiter references.
Uses published SBR equations with ISO Cor.1 epsilon=1 and S_IndexMapped sine.
"""
from decimal import Decimal,localcontext
from pathlib import Path
import json
D=Decimal
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
def levels(bands,suppress):
    out=[]
    for target,current,q,presence,line in bands:
        inverse=1/(1+q);noise=q/(1+q)
        fraction=noise if presence else D(1) if suppress else inverse
        out.append([(target*fraction/(1+current)).sqrt(),(target*noise).sqrt(),(target*inverse).sqrt() if line else D(0)])
    return out
def limit(bands,values,borders,mode,suppress):
    factor=[D(".70795"),D(1),D("1.41254"),D("1e10")][mode]
    out=[v[:] for v in values]
    for lo,hi in zip(borders,borders[1:]):
        start,end=lo-10,hi-10
        original=sum(b[0] for b in bands[start:end])+D("1e-12")
        current=sum(b[1] for b in bands[start:end])+D("1e-12")
        maximum=min((original/current).sqrt()*factor,D("1e5"))
        for v in out[start:end]:
            if v[0]>maximum:v[1]*=maximum/v[0];v[0]=maximum
        reconstructed=D("1e-12")
        for b,v in zip(bands[start:end],out[start:end]):
            reconstructed+=b[1]*v[0]**2+v[2]**2
            if not suppress and v[2]==0:reconstructed+=v[1]**2
        boost=min((original/reconstructed).sqrt(),D("1.584893192"))
        for i in range(start,end):out[i]=[x*boost for x in out[i]]
    return out
def serialized(bands):return [[float(t),float(c),float(q),p,l] for t,c,q,p,l in bands]
cases=[]
with localcontext() as ctx:
    ctx.prec=80
    for raw in [(0,0,0),(4,10,.5),(512,16,64),(64,0,0),(1e-12,1e-8,.25),(1e4,200,8)]:
        target,current,q=[D.from_float(float(x)) for x in raw]
        for presence,line in [(False,False),(True,False),(True,True)]:
            bands=[(target,current,q,presence,line)]
            for suppress in [False,True]:
                expected=levels(bands,suppress)
                cases.append({"bands":serialized(bands),"suppress":suppress,"expected":[list(map(float,v)) for v in expected]})
    bands=[(D(2)**(k%8+4),D(0) if k==0 else D(2)**(k%6+2),D(2)**(k%7-3),k//4%2==1,k//4%2==1 and k%4==2) for k in range(16)]
    for borders in [[10,26],[10,14,20,26],[10,11,15,17,24,26]]:
        for mode in range(4):
            for suppress in [False,True]:
                expected=limit(bands,levels(bands,suppress),borders,mode,suppress)
                cases.append({"bands":serialized(bands),"suppress":suppress,"borders":borders,"mode":mode,"expected":[list(map(float,v)) for v in expected]})
assert len(cases)==60
(root/"aac-sbr-gain-decimal.json").write_text(json.dumps({"source":"GOST R53556.4-2013 6.18.7.4/5; ISO/IEC 14496-3:2001/Amd.1:2003/Cor.1:2004 pp4,11", "precision":80,"cases":cases},separators=(",",":"))+"\n")
