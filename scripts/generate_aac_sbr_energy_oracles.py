#!/usr/bin/env python3
"""Offline 80-digit energy oracle for original complex QMF and saved HF traces."""
from decimal import Decimal,localcontext
from pathlib import Path
import json,struct,hashlib
D=Decimal
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
source=(root/"aac-sbr-hf-decimal.f64le").read_bytes()
source_offset=0;payload=bytearray()
with localcontext() as ctx:
    ctx.prec=80
    for slots in [15,16]:
        source_offset+=(2*slots+8)*32*16
        hf=[]
        for t in range(2*slots+6):
            row=[]
            for k in range(64):
                a,b=struct.unpack_from("<dd",source,source_offset);source_offset+=16
                row.append((D.from_float(a),D.from_float(b)))
            hf.append(row)
        for case in range(5):
            high=[]
            for t in range(2*slots+6):
                row=[]
                for k in range(64):
                    value=[(D(0),D(0)),(D((t*7+k*11)%31-15)/16,D((t*13+k*3)%29-14)/32),
                           (D(4) if t==8 and k==10 else D(0),D(0)),
                           (D(k+1)/16,D(k%5)/32),hf[t][k]][case]
                    row.append(value);payload.extend(struct.pack("<dd",float(value[0]),float(value[1])))
                high.append(row)
            time=[1,4,9,slots+2];fine=[False,True,False]
            for interpolate in [False,True]:
                for envelope in range(3):
                    borders=list(range(10,29)) if interpolate else [10,11,14,18,21,28] if fine[envelope] else [10,11,18,28]
                    for start,end in zip(borders,borders[1:]):
                        values=[a*a+b*b for row in high[2*time[envelope]:2*time[envelope+1]] for a,b in row[start:end]]
                        energy=sum(values)/D(len(values))
                        for _ in range(end-start):payload.extend(struct.pack("<d",float(energy)))
assert source_offset==len(source)
name="aac-sbr-energy-decimal.f64le"
(root/name).write_bytes(payload)
(root/"aac-sbr-energy-oracles.json").write_text(json.dumps({"source":"GOST R53556.4-2013 6.18.7.3", "precision":80,"slots":[15,16],"cases":["silence","dense rational","boundary impulse","constant bands","independent HF output"],"high":[10,11,14,18,21,28],"low":[10,11,18,28],"resolution":[False,True,False],"hf_dependency_sha256":hashlib.sha256(source).hexdigest(),"file":name,"sha256":hashlib.sha256(payload).hexdigest()},indent=2)+"\n")
