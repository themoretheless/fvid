"""Owned nonseparate H.264 4:4:4 intra loop-filter witnesses, no external codecs."""
import itertools, json, struct
from pathlib import Path
from generate_avc_pcm_formats import Writer, configuration as base_configuration, mux
from generate_avc_separate_8x8_deblock import filter_edge_sample, residual_8x8_single
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-deblock'


def configuration(yd,cd,offsets):
    cfg=base_configuration(3,yd,cd,True,height_mbs=2,level=41)
    n=int.from_bytes(cfg[6:8],'big')
    b=Writer();b.ue(0);b.ue(0);b.u(0);b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(offsets[0]);b.u(1);b.u(0);b.u(0)
    b.u(1);b.u(0);b.se(offsets[1]) # transform8 enabled, no custom matrices
    pps=b.nal(0x68)
    return cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps


def chroma_qp(qp,offset,depth):
    q=max(-6*(depth-8),min(51,qp+offset))
    return q if q<30 else [29,30,31,32,32,33,34,34,35,35,36,36,37,37,37,38,38,38,39,39,39,39][q-30]


def coefficient(b,level,zeros):
    if abs(level)==1:
        b.u(1,2);b.u(int(level<0))
    else:
        assert abs(level)==4
        b.u(5,6);b.u(1,6 if level<0 else 5)
    code,bits=[(1,1),(3,3),(2,3),(3,4),(2,4),(3,5)][zeros]
    b.u(code,bits)


def i16_samples(depth,qp,level,scan):
    # A single coefficient times the two explicit integer Hadamard basis vectors.
    position=[0,1,4,8,5,2][scan]
    basis=[[1,1,1,1],[1,1,-1,-1],[1,-1,-1,1],[1,-1,1,-1]]
    prime=qp+6*(depth-8)
    norm=[10,11,13,14,16,18][prime%6]*16
    values=[]
    for y in range(16):
        for x in range(16):
            v=level*basis[position//4][y//4]*basis[position%4][x//4]*norm
            dc=v<<(prime//6-6) if prime//6>=6 else (v+(1<<(5-prime//6)))>>(6-prime//6)
            values.append(max(0,min((1<<depth)-1,(1<<(depth-1))+((dc+32)>>6))))
    return values


def i8_samples(depth,qp,sign):
    residual=residual_8x8_single(0,4*sign,qp,depth,16)[0]
    b0=max(0,min((1<<depth)-1,(1<<(depth-1))+residual))
    b1=max(0,min((1<<depth)-1,b0+residual))
    t7=(3*b0+b1+2)//4
    b2=max(0,min((1<<depth)-1,(7*b0+t7+4)//8+residual))
    t0=(b0+3*b1+2)//4;l0=(b0+3*b2+2)//4
    b3=max(0,min((1<<depth)-1,(t0+7*b1+l0+7*b2+8)//16+residual))
    vals=[b0,b1,b2,b3]
    return [vals[y//8*2+x//8]for y in range(16)for x in range(16)]


def filtered(samples,depth,qps,mode,offset,eight,wrong_grid=False,shared_slice=False,field=False):
    out=samples[:]
    if mode==1:return out
    for mb in range(4):
        mx,my=mb%2,mb//2
        for vertical in [True,False]:
            for edge in range(0,16,4):
                if eight and not wrong_grid and edge%8:continue
                if edge==0 and ((mx==0 if vertical else my==0)or (mode==2 and not shared_slice)):continue
                qp=(qps[mb]+qps[mb-(1 if vertical else 2)]+1)//2 if edge==0 else qps[mb]
                for line in range(16):
                    x=mx*16+(edge if vertical else line);y=my*16+(line if vertical else edge)
                    at=y*32+x;step=1 if vertical else 32
                    p=[out[at-(j+1)*step]for j in range(4)];q=[out[at+j*step]for j in range(4)]
                    a,b=filter_edge_sample(p,q,qp+2*offset,depth,4 if edge==0 and (vertical or not field) else 3)
                    for j in range(3):out[at-(j+1)*step]=a[j];out[at+j*step]=b[j]
    return out


def generate():
    OUT.mkdir(exist_ok=True);cases=[];changed=[0,0,0];qp_witness=0;grid_witness=0
    for (yd,cd),offsets,mode,filter_offset,signal in itertools.product(
        [(8,8),(10,12),(14,8)],[(0,0),(-6,6),(6,-6)],[0,1,2],[-3,0,3],['strong','short','internal','i8']):
        name=f'y{yd}-c{cd}-o{offsets[0]}-{offsets[1]}-m{mode}-f{filter_offset}-{signal}'
        config=configuration(yd,cd,offsets);nals=[];blocks=[[],[],[]];qps=[[],[],[]]
        for mb in range(4):
            qp=[26,30,30,26][mb]
            b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(qp-26);b.ue(mode)
            if mode!=1:b.se(filter_offset);b.se(filter_offset)
            eight=signal=='i8'
            b.ue(0 if eight else 3)
            if eight:
                b.u(1)
                for _ in range(4):b.u(1)
                b.ue(0)
            b.se(0)
            for p,depth in enumerate([yd,cd,cd]):
                component_qp=qp if p==0 else chroma_qp(qp,offsets[p-1],depth)
                qps[p].append(component_qp)
                sign=-1 if (mb+p)%2==0 else 1
                if eight:
                    for sub in range(16):
                        if sub%4==0:coefficient(b,4*sign,0)
                        else:b.u(1)
                    block=i8_samples(depth,component_qp,sign)
                else:
                    level=sign*(4 if signal=='short' else 1)
                    scan=[5,3,4][p]if signal=='internal' else 0
                    coefficient(b,level,scan)
                    block=i16_samples(depth,component_qp,level,scan)
                blocks[p].append(block)
            nal=b.nal(0x65);nals.append(len(nal).to_bytes(4,'big')+nal)
        planes=[];counts=[]
        for p,depth in enumerate([yd,cd,cd]):
            raw=[blocks[p][y//16*2+x//16][y%16*16+x%16]for y in range(32)for x in range(32)]
            expected=filtered(raw,depth,qps[p],mode,filter_offset,eight)
            count=sum(a!=b for a,b in zip(raw,expected));counts.append(count);changed[p]+=count
            if p:qp_witness+=sum(a!=b for a,b in zip(expected,filtered(raw,depth,qps[0],mode,filter_offset,eight)))
            if eight:grid_witness+=sum(a!=b for a,b in zip(expected,filtered(raw,depth,qps[p],mode,filter_offset,eight,True)))
            if mode==1:assert raw==expected
            planes+=expected
        packet=b''.join(nals)
        for ext,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,32,25)),('y16',struct.pack('<3072H',*planes))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,mode=mode,changed=counts))
    assert len(cases)==324 and min(changed)>0 and qp_witness>0 and grid_witness>0
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance=__doc__,cases=cases,changed=changed,wrong_chroma_qp_witness=qp_witness,wrong_8x8_grid_witness=grid_witness),indent=2)+'\n')
    print('cases',len(cases),'changed',changed,'QP/grid witnesses',qp_witness,grid_witness)
if __name__=='__main__':generate()
