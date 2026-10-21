"""Owned shared-slice PCM/I16 4:4:4 CABAC filtering and arithmetic-restart witnesses."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_deblock import configuration,chroma_qp,i16_samples,filtered
from generate_avc_pcm_formats import Writer,mux
from avc_cabac_444_writer import Cabac444Writer
from generate_avc_444_cabac_intra import single
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-mixed'
def generate():
    OUT.mkdir(exist_ok=True);cases=[];changed=[0,0,0]
    for (yd,cd),offsets,mode in itertools.product([(8,8),(10,12),(14,8)],[(12,12),(12,-12),(-12,12)],[0,1,2]):
        name=f'y{yd}-c{cd}-o{offsets[0]}-{offsets[1]}-m{mode}'
        cfg=configuration(yd,cd,offsets)
        n=int.from_bytes(cfg[6:8],'big');p=Writer();p.ue(0);p.ue(0);p.u(1);p.u(0);p.ue(0);p.ue(0);p.ue(0);p.u(0);p.u(0,2)
        p.se(0);p.se(0);p.se(offsets[0]);p.u(1);p.u(0);p.u(0);p.u(1);p.u(0);p.se(offsets[1]);pps=p.nal(0x68)
        cfg=cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps
        planes=[[((80+20*p)+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(32)for x in range(32)]for p,depth in enumerate([yd,cd,cd])]
        qps=[[],[],[]]
        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(mode)
        if mode!=1:b.se(6);b.se(6)
        while len(b.bits)%8:b.u(1)
        c=Cabac444Writer()
        for mb in range(4):
            pcm=mb in [0,3];c.decision(3+[0,1,1,2][mb],1)
            if pcm:b.bits.extend(c.finish());b.align()
            else:
                c.range-=2;c.renormalize()
                for ctx,value in [(6,0),(7,0),(9,1),(10,0),(60,0)]:c.decision(ctx,value)
            for p,depth in enumerate([yd,cd,cd]):
                qpy=0 if pcm else 26
                qp=qpy if p==0 else chroma_qp(qpy,offsets[p-1],depth)
                qps[p].append(qp)
                if pcm:
                    for y in range(16):
                        for x in range(16):b.u(planes[p][(mb//2*16+y)*32+mb%2*16+x],depth)
                else:
                    level=-1 if (p+mb)%2 else 1
                    # Both DC neighbours are PCM/intra-unavailable -> condTerm A=B=1.
                    single(c,[0,6,10][p],3,level)
                    residual=i16_samples(depth,qp,level,0)
                    edge=[planes[p][y*32+15]for y in range(16)]if mb==1 else planes[p][15*32:15*32+16]
                    pred=(sum(edge)+8)//16
                    for y in range(16):
                        for x in range(16):planes[p][(mb//2*16+y)*32+mb%2*16+x]=max(0,min((1<<depth)-1,pred+residual[y*16+x]-(1<<(depth-1))))
            if pcm:c.low=0;c.range=510;c.width=9
            if mb<3:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
        expected=[];counts=[]
        for p,depth in enumerate([yd,cd,cd]):
            out=filtered(planes[p],depth,qps[p],mode,6,False,shared_slice=True)
            n=sum(a!=b for a,b in zip(out,planes[p]));changed[p]+=n;counts.append(n);expected+=out
        if mode==1:assert counts==[0,0,0]
        nal=b.nal(0x65,trailing=False);packet=len(nal).to_bytes(4,'big')+nal
        for ext,data in [('avcc',cfg),('packet',packet),('mp4',mux(cfg,[(0,True,packet)],32,32,25)),('y16',struct.pack('<3072H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,mode=mode,changed=counts))
    assert len(cases)==27 and min(changed)>0
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases,changed=changed),indent=2)+'\n')
    print('mixed cases',len(cases),'changed',changed)
if __name__=='__main__':generate()
