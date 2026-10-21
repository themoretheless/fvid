"""Owned nonseparate 4:4:4 PCM filtering witnesses in CAVLC and CABAC."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_deblock import configuration,chroma_qp,filtered
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-pcm-deblock'

def generate():
    OUT.mkdir(exist_ok=True);cases=[];changed=[0,0,0]
    for (yd,cd),offsets,mode,cabac in itertools.product([(8,8),(10,12),(14,8)],[(12,12),(12,-12),(-12,12)],[0,1,2],[False,True]):
        name=f'y{yd}-c{cd}-o{offsets[0]}-{offsets[1]}-m{mode}-e{int(cabac)}'
        cfg=configuration(yd,cd,offsets)
        # Replace PPS solely to select the entropy mode. All owned SPS fields remain identical.
        n=int.from_bytes(cfg[6:8],'big');b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
        b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0);b.u(1);b.u(0);b.se(offsets[1]);pps=b.nal(0x68)
        cfg=cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps
        planes=[[((80+20*p)+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(32)for x in range(32)]for p,depth in enumerate([yd,cd,cd])]
        nals=[]
        for mb in range(4):
            b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(mode)
            if mode!=1:b.se(6);b.se(6)
            if cabac:
                while len(b.bits)%8:b.u(1)
                c=CabacWriter(-1,26);c.decision(3,1);b.bits.extend(c.finish());b.align()
            else:b.ue(25);b.align()
            for p,depth in enumerate([yd,cd,cd]):
                for y in range(16):
                    for x in range(16):b.u(planes[p][(mb//2*16+y)*32+mb%2*16+x],depth)
            if cabac:
                c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
            nal=b.nal(0x65,trailing=not cabac);nals.append(len(nal).to_bytes(4,'big')+nal)
        expected=[];counts=[]
        for p,depth in enumerate([yd,cd,cd]):
            qp=0 if p==0 else chroma_qp(0,offsets[p-1],depth)
            out=filtered(planes[p],depth,[qp]*4,mode,6,False)
            count=sum(a!=b for a,b in zip(out,planes[p]));counts.append(count);changed[p]+=count;expected+=out
        assert counts[0]==0
        if mode==1:assert counts==[0,0,0]
        else:assert counts[1]+counts[2]>0
        packet=b''.join(nals)
        for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,32,25)),('y16',struct.pack('<3072H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,mode=mode,changed=counts))
    assert len(cases)==54 and changed[1]>0 and changed[2]>0
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases,changed=changed),indent=2)+'\n')
    print('PCM cases',len(cases),'changed',changed)
if __name__=='__main__':generate()
