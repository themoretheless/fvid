"""Owned monochrome CABAC intra signed-DC and PCM witnesses, offline."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-monochrome-cabac'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth in [8,10,14]:
        for kind in ['zero','positive','negative','pcm','i4-zero']:
            name=f'mono-cabac-{depth}-{kind}';config=configuration(0,depth,8,False,True);nals=[];planes=[]
            for address in range(2):
                b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(24);b.ue(1)
                while len(b.bits)%8:b.u(1)
                c=CabacWriter(-1,50);c.decision(3,int(kind!='i4-zero'))
                if kind=='pcm':
                    b.bits.extend(c.finish());b.align()
                    samples=[(x*17+y*23+address*41)%(1<<depth)for y in range(16)for x in range(16)]
                    for v in samples:b.u(v,depth)
                    c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
                elif kind=='i4-zero':
                    for _ in range(16):c.decision(68,1)
                    for ctx in [73,74,75,76]:c.decision(ctx,0)
                    b.bits.extend(c.finish());samples=[1<<(depth-1)]*256
                else:
                    c.range-=2;c.renormalize()
                    c.decision(6,0);c.decision(7,0);c.decision(9,1);c.decision(10,0)
                    c.decision(60,0);c.decision(88,int(kind!='zero'))
                    sign=-1 if (kind=='negative') ^ bool(address) else 1
                    if kind!='zero':
                        c.decision(105,1);c.decision(166,1);c.decision(228,0);c.bypass(int(sign<0))
                    b.bits.extend(c.finish())
                    # Flat scaling16, QP'=50+6*(depth-8), single DC level.
                    qp=50+6*(depth-8);dc=sign*13*16*(1<<(qp//6-6)) if kind!='zero' else 0
                    value=(1<<(depth-1))+((dc+32)>>6);samples=[value]*256
                planes.append(samples);nals.append(b.nal(0x65,trailing=False))
            expected=[v for y in range(16)for p in planes for v in p[y*16:y*16+16]]
            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
            for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,16,25)),('y16',struct.pack('<512H',*expected))]:(OUT/(name+'.'+suffix)).write_bytes(data)
            cases.append(dict(name=name,depth=depth,kind=kind))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
