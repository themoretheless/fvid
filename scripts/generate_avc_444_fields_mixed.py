"""Owned shared-slice 4:4:4 CAVLC/CABAC PCM/I16 field pairs with physical parity oracles."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_fields import configuration,cabac_single
from generate_avc_444_deblock import chroma_qp,i16_samples,filtered
from generate_avc_pcm_formats import Writer,mux
from avc_cabac_444_writer import Cabac444Writer
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-fields-mixed'


def field(yd,cd,cabac,offsets,mode,bottom,index):
    planes=[[((80+20*p+int(bottom)*7)+2*((x//4+y//4)%2))*(1<<(depth-8))for y in range(32)for x in range(32)]for p,depth in enumerate([yd,cd,cd])]
    qps=[[],[],[]];b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(1);b.u(int(bottom))
    if index==0:b.ue(0)
    b.u(index,4)
    if index==0:b.u(0);b.u(0)
    else:b.u(0)
    b.se(0);b.ue(mode)
    if mode!=1:b.se(6);b.se(6)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=Cabac444Writer()
    for mb in range(4):
        pcm=mb in [0,3]
        if cabac:
            c.decision(3+[0,1,1,2][mb],1)
            if pcm:b.bits.extend(c.finish());b.align()
            else:
                c.range-=2;c.renormalize()
                for ctx,value in [(6,0),(7,0),(9,1),(10,0),(60,0)]:c.decision(ctx,value)
        else:
            b.ue(25 if pcm else 3)
            if pcm:b.align()
            else:b.se(0)
        for p,depth in enumerate([yd,cd,cd]):
            qpy=0 if pcm else 26
            qp=qpy if p==0 else chroma_qp(qpy,offsets[p-1],depth);qps[p].append(qp)
            if pcm:
                for y in range(16):
                    for x in range(16):b.u(planes[p][(mb//2*16+y)*32+mb%2*16+x],depth)
            else:
                level=-1 if (p+mb+int(bottom))%2 else 1
                if cabac:cabac_single(c,[0,6,10][p],0,level)
                else:b.u(1,6);b.u(int(level<0));b.u(1) # nC=16 from PCM, separately in each plane
                residual=i16_samples(depth,qp,level,0)
                edge=[planes[p][y*32+15]for y in range(16)]if mb==1 else planes[p][15*32:15*32+16]
                pred=(sum(edge)+8)//16
                for y in range(16):
                    for x in range(16):planes[p][(mb//2*16+y)*32+mb%2*16+x]=max(0,min((1<<depth)-1,pred+residual[y*16+x]-(1<<(depth-1))))
        if cabac:
            if pcm:c.low=0;c.range=510;c.width=9
            if mb<3:c.range-=2;c.renormalize()
    if cabac:b.bits.extend(c.finish())
    nal=b.nal(0x65 if index==0 else 0x41,trailing=not cabac)
    expected=[filtered(planes[p],depth,qps[p],mode,6,False,shared_slice=True,field=True)for p,depth in enumerate([yd,cd,cd])]
    return len(nal).to_bytes(4,'big')+nal,expected


def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,bottom_first,offsets,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],[False,True],[(0,0),(-12,12),(12,-12)],[0,1,2]):
        name=f'y{yd}-c{cd}-e{int(cabac)}-b{int(bottom_first)}-o{offsets[0]}-{offsets[1]}-f{mode}'
        cfg=configuration(yd,cd,cabac,offsets);packets=[];physical={}
        for index,bottom in enumerate([True,False]if bottom_first else[False,True]):
            packet,planes=field(yd,cd,cabac,offsets,mode,bottom,index);packets.append(packet);physical[bottom]=planes
            (OUT/f'{name}-{index}.packet').write_bytes(packet)
        assert physical[False]!=physical[True]
        woven=[]
        for p in range(3):
            for y in range(32):
                for bottom in [False,True]:woven+=physical[bottom][p][y*32:(y+1)*32]
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,50)),('y16',struct.pack('<6144H',*woven))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,pcm=False))
    assert len(cases)==108
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases),indent=2)+'\n')
    print('mixed field cases',len(cases))
if __name__=='__main__':generate()
