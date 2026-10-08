#!/usr/bin/env python3
"""Explicit offline Decimal oracle: original QMF signals, covariance and HF."""
from decimal import Decimal, localcontext
from pathlib import Path
import struct, json, hashlib
D=Decimal
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
def mul(a,b):return (a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0])
def conj(a):return (a[0],-a[1])
def predictor(x):
    def phi(i,j):
        terms=[mul(x[n-i],conj(x[n-j])) for n in range(2,len(x))]
        return tuple(sum(t[k] for t in terms) for k in range(2))
    a,b,c=phi(0,1),phi(0,2),phi(1,2)
    y,z=phi(1,1)[0],phi(2,2)[0]
    det=y*z-(c[0]**2+c[1]**2)/(1+D("0.000001"))
    p=mul(a,c)
    second=tuple((p[k]-b[k]*y)/det for k in range(2)) if det else (D(0),D(0))
    p=mul(second,conj(c))
    first=tuple(-(a[k]+p[k])/y for k in range(2)) if y else (D(0),D(0))
    if sum(v*v for v in first)>=16 or sum(v*v for v in second)>=16:first=second=(D(0),D(0))
    return first,second
payload=bytearray()
with localcontext() as ctx:
    ctx.prec=80
    for slots in [15,16]:
        low=[[(D((t*17+p*7)%29-14)/16,D((t*13+p*11)%31-15)/32) for p in range(32)] for t in range(2*slots+8)]
        for row in low:
            for a,b in row:payload.extend(struct.pack("<dd",float(a),float(b)))
        predictors=[predictor([r[p] for r in low]) for p in range(10)]
        mapping={**{k:k-8 for k in range(10,18)},**{k:k-16 for k in range(18,26)}}
        for t in range(2*slots+6):
            for k in range(64):
                value=(D(0),D(0))
                if k in mapping and 2<=t<2*(slots+2):
                    p=mapping[k];bw=D("0.6") if k<17 else D("0.9")
                    first,second=predictors[p]
                    a=mul(first,low[t+1][p]);b=mul(second,low[t][p])
                    value=tuple(low[t+2][p][j]+bw*a[j]+bw*bw*b[j] for j in range(2))
                payload.extend(struct.pack("<dd",float(value[0]),float(value[1])))
name="aac-sbr-hf-decimal.f64le"
(root/name).write_bytes(payload)
(root/"aac-sbr-hf-oracles.json").write_text(json.dumps({"source":"GOST R53556.4-2013 6.18.6.3", "precision":80,"slots":[15,16],"patches":[[2,10,8],[2,18,8]],"noise":[10,17,28],"chirp":[0.6,0.9],"file":name,"sha256":hashlib.sha256(payload).hexdigest()},indent=2)+"\n")
