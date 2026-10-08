#!/usr/bin/env python3
"""Offline, original complex signals and 80-digit covariance equation oracle."""
from decimal import Decimal, localcontext
from pathlib import Path
import struct, json, hashlib
D=Decimal
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
def mul(a,b):return (a[0]*b[0]-a[1]*b[1],a[0]*b[1]+a[1]*b[0])
def conj(a):return (a[0],-a[1])
def phi(x,i,j):
    values=[mul(x[n-i],conj(x[n-j])) for n in range(2,len(x))]
    return tuple(sum(v[k] for v in values) for k in range(2))
data=bytearray()
with localcontext() as ctx:
    ctx.prec=80
    for slots in [15,16]:
        for case in range(6):
            values=[]
            for n in range(2*slots+8):
                a,b=[(0,0),(1,0),(1,-2),((n*17%29-14)/16,(n*13%31-15)/32),(n/32,1-n/64),(5**n,0)][case]
                values.append((float(a),float(b)))
            x=[(D.from_float(a),D.from_float(b)) for a,b in values]
            p01,p02,p12=phi(x,0,1),phi(x,0,2),phi(x,1,2)
            p11,p22=phi(x,1,1)[0],phi(x,2,2)[0]
            det=p11*p22-(p12[0]**2+p12[1]**2)/(1+D("0.000001"))
            product=mul(p01,p12)
            second=tuple((product[k]-p02[k]*p11)/det for k in range(2)) if det else (D(0),D(0))
            product=mul(second,conj(p12))
            first=tuple(-(p01[k]+product[k])/p11 for k in range(2)) if p11 else (D(0),D(0))
            if sum(v*v for v in first)>=16 or sum(v*v for v in second)>=16:first=second=(D(0),D(0))
            for a,b in values:data.extend(struct.pack("<dd",a,b))
            for v in (*first,*second):data.extend(struct.pack("<d",float(v)))
name="aac-sbr-predictor-decimal.f64le"
(root/name).write_bytes(data)
(root/"aac-sbr-predictor-oracles.json").write_text(json.dumps({"source":"GOST R53556.4-2013 6.18.6.2; ISO working draft epsilon=1e-6", "precision":80,"slots":[15,16],"cases":["zero","real constant","complex constant","dense rational","complex ramp","unstable growth"],"file":name,"sha256":hashlib.sha256(data).hexdigest()},indent=2)+"\n")
