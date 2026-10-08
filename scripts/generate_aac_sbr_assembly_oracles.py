#!/usr/bin/env python3
"""Offline original multiframe SBR assembly traces using direct Decimal sums."""
from pathlib import Path
from decimal import Decimal,localcontext
import struct,json,hashlib
D=Decimal
root=Path(__file__).resolve().parents[1]/"tests/fixtures/playback-errors"
noise_source=(root/'aac-sbr-noise-protocol.f64le').read_bytes()
noise=[tuple(D.from_float(x) for x in struct.unpack_from('<dd',noise_source,i*16)) for i in range(512)]
weights=list(map(D,['0.33333333333333','0.30150283239582','0.21816949906249','0.11516383427084','0.03183050093751']))
metadata=[];payload=bytearray()
with localcontext() as ctx:
    ctx.prec=80
    for slots in [15,16]:
        for disabled in [False,True]:
            history=None;noise_start=0;sine_start=0;frames=[]
            controls=[[0,4,slots],[1,6,slots+1],[0,7,slots+2],[1,8,slots],[0,5,slots]]
            for frame,envelope in enumerate(controls):
                reset=frame==4;suppress=[frame==1,frame==2]
                levels=[[(D(frame+1)/2+D(env+1)/4+D(m)/16,D(env+1)/8+D(frame)/16+D(m)/32,D('.75') if m in [1,3] and (frame+env)%2==0 else D(0)) for m in range(5)] for env in range(2)]
                start,end=2*envelope[0],2*envelope[-1]
                raw=[levels[0] if t<2*envelope[1] else levels[1] for t in range(start,end)]
                if reset or history is None:history=[levels[0]]*4;noise_start=0
                sequence=history+raw
                high=[[(D((t*7+m*3+frame)%17-8)/16,D((t*11+m*5+frame)%19-9)/32) for m in range(5)] for t in range(2*slots+6)]
                for row in high:
                    for a,b in row:payload.extend(struct.pack('<dd',float(a),float(b)))
                for t,row in enumerate(high):
                    for m,(re,im) in enumerate(row):
                        value=(D(0),D(0))
                        if start<=t<end:
                            env=0 if t<2*envelope[1] else 1
                            pos=4+t-start;g,q,s=sequence[pos][m]
                            gain=g if disabled or suppress[env] else sum(weights[j]*sequence[pos-j][m][0] for j in range(5))
                            q=D(0) if suppress[env] or s else q if disabled else sum(weights[j]*sequence[pos-j][m][1] for j in range(5))
                            random=noise[(noise_start+(t-start)*5+m+1)%512]
                            phase=(sine_start+t-start)%4
                            sine_re=[1,0,-1,0][phase];sine_im=[0,1,0,-1][phase]*(-1 if (11+m)%2 else 1)
                            value=(gain*re+q*random[0]+s*D(sine_re),gain*im+q*random[1]+s*D(sine_im))
                        payload.extend(struct.pack('<dd',float(value[0]),float(value[1])))
                history=sequence[-4:];noise_start=(noise_start+(end-start)*5)%512;sine_start=(sine_start+end-start)%4
                frames.append({'envelope':envelope,'levels':[[list(map(float,x)) for x in r] for r in levels],'suppress':suppress,'reset':reset,'noise_end':noise_start,'sine_end':sine_start})
            metadata.append({'slots':slots,'disabled':disabled,'frames':frames})
name='aac-sbr-assembly-decimal.f64le'
(root/name).write_bytes(payload)
(root/'aac-sbr-assembly-oracles.json').write_text(json.dumps({'source':'GOST R53556.4-2013 6.18.7.6/A.91','precision':80,'noise_sha256':hashlib.sha256(noise_source).hexdigest(),'file':name,'sha256':hashlib.sha256(payload).hexdigest(),'cases':metadata},indent=2)+'\n')
