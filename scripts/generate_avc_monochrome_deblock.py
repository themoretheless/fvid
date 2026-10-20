"""Owned monochrome CABAC I16 cross-slice deblocking witnesses, offline."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-monochrome-deblock'
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth,cabac in [(d,c)for d in [8,10,14]for c in [False,True]]:
        for mode in [0,1,2]:
            name=f'mono-filter-{depth}-{mode}-'+('cabac'if cabac else'cavlc');config=configuration(0,depth,8,False,cabac);nals=[];values=[]
            for address in range(2):
                b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(mode)
                if mode!=1:b.se(0);b.se(0)
                if not cabac:
                    b.ue(3);b.se(0) # I16 DC, QP unchanged, no chroma syntax
                    b.u(5,6) # TotalCoeff1, TrailingOnes0 at nC0
                    b.u(1,6 if address==0 else 5) # level -4 / +4
                    b.u(1) # total_zeros0
                    nals.append(b.nal(0x65))
                else:
                    while len(b.bits)%8:b.u(1)
                    c=CabacWriter(-1,26);c.decision(3,1);c.range-=2;c.renormalize()
                    for ctx,value in [(6,0),(7,0),(9,1),(10,0),(60,0),(88,1),(105,1),(166,1),(228,1),(232,1),(232,1),(232,0)]:c.decision(ctx,value)
                    c.bypass(int(address==0));b.bits.extend(c.finish());nals.append(b.nal(0x65,trailing=False))
                qp=26+6*(depth-8);v=4*13*16
                dc=v<<(qp//6-6) if qp//6>=6 else (v+(1<<(5-qp//6)))>>(6-qp//6)
                signed=dc if address else -dc
                values.append((1<<(depth-1))+((signed+32)>>6))
            expected=[v for _ in range(16) for v in [values[0]]*16+[values[1]]*16]
            if mode==0:
                # Flat intra edge, bS4; alpha=15*scale, beta=6*scale.
                # Difference exceeds alpha/4+2, selecting the short filter.
                scale=1<<(depth-8);assert abs(values[0]-values[1])<15*scale
                assert abs(values[0]-values[1])>=((15*scale)>>2)+2
                for y in range(16):
                    expected[y*32+15]=(3*values[0]+values[1]+2)//4
                    expected[y*32+16]=(3*values[1]+values[0]+2)//4
                assert expected!=[v for _ in range(16)for v in [values[0]]*16+[values[1]]*16]
            packet=b''.join(len(n).to_bytes(4,'big')+n for n in nals)
            for suffix,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,16,25)),('y16',struct.pack('<512H',*expected))]:(OUT/(name+'.'+suffix)).write_bytes(data)
            cases.append(dict(name=name,depth=depth,mode=mode,cabac=cabac))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
