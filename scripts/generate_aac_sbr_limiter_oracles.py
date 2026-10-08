#!/usr/bin/env python3
"""Offline original limiter geometries; Decimal ratio comparison oracle.
Figure 4.40 uses log2(ratio)*density < .49; range endpoints are retained
so the Cor.1 gain k(m) exists for every m<M even after a short patch discard.
This oracle instead compares ratio
with the high-precision exponential threshold, without floating logarithms.
"""
from decimal import Decimal, localcontext
from pathlib import Path
import json
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
def reference(low,patches,mode,threshold):
    if any(p[0]+p[2]>min(low[0],32) for p in patches):return None
    if mode==0:return [low[0],low[-1]]
    protected={low[0],low[-1],*(p[1]+p[2] for p in patches)}
    values=sorted(low+[p[1] for p in patches[1:]])
    cursor=1
    while cursor<len(values):
        left,right=values[cursor-1:cursor+1]
        if Decimal(right)/Decimal(left)>=threshold[mode]:cursor+=1;continue
        remove=cursor if left==right or right not in protected else cursor-1 if left not in protected else None
        if remove is None:cursor+=1
        else:values.pop(remove)
    return values if len(values)>1 else None
cases=[]
with localcontext() as ctx:
    ctx.prec=80
    thresholds={mode:Decimal(2)**(Decimal(".49")/density) for mode,density in enumerate([Decimal("1.2"),Decimal(2),Decimal(3)],1)}
    for start in [10,16,20,25,32]:
        for widths in [[4],[8],[16],[4,4,4],[8,8],[6,4,8],[4,6,4,8]]:
            for tail in range(3):
                end=start+sum(widths)+tail
                if end>64:continue
                patches=[];target=start
                for width in widths:
                    source=target%2
                    patches.append([source,target,width]);target+=width
                for step in [1,2,3,5]:
                    low=list(range(start,end,step))+[end]
                    for mode in range(4):
                        cases.append({"low":low,"patches":patches,"mode":mode,"expected":reference(low,patches,mode,thresholds)})
(root/"aac-sbr-limiter-decimal.json").write_text(json.dumps({"source":"ISO SBR Figure 4.40 and Cor.1 4.6.18.7.5 covering k(m); GOST R53556.4-2013 6.18.3.2.3", "precision":80,"cases":cases},separators=(",",":"))+"\n")
print(len(cases),"original limiter geometries")
