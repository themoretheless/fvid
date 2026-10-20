"""Owned B bi filtering: independently vary L0 and L1 motion at a border."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import Writer,configuration,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
ROOT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors'
BASE=ROOT/'avc-422-inter-deblock';OUT=ROOT/'avc-422-b-deblock'
def future_pcm(source,yd,cd,cabac):
    nals=[]
    for mb in range(4):
        b=Writer();b.ue(mb);b.ue(2);b.ue(0);b.u(1,4);b.u(6,4);b.u(0);b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(3,1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for at,w,bw,depth in [(0,32,16,yd),(1024,16,8,cd),(1536,16,8,cd)]:
            for y in range(16):
                for x in range(bw):b.u(source[at+(mb//2*16+y)*w+mb%2*bw+x],depth)
        if cabac:c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
        nals.append(b.nal(0x41,trailing=not cabac))
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def middle_packet(cabac,axis,motion_delta,mode,changed_list):
    nals=[]
    for mb in range(4):
        b=Writer();b.ue(mb);b.ue(1);b.ue(0);b.u(2,4);b.u(2,4);b.u(1);b.u(0);b.u(0);b.u(0)
        if cabac:b.ue(0)
        b.se(0);b.ue(mode)
        if mode!=1:b.se(0);b.se(0)
        vectors=[[0,0],[0,0]];vectors[changed_list][axis]=motion_delta*[mb%2,mb//2][axis]
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,26);c.decision(24,0);mb_type(c,3,0)
            for v in vectors:c.mvd(0,v[0]);c.mvd(1,v[1])
            for ctx in [73,74,75,76,77]:c.decision(ctx,0)
            b.bits.extend(c.finish())
        else:
            b.ue(0);b.ue(3)
            for v in vectors:b.se(v[0]);b.se(v[1])
            b.ue(0)
        nals.append(b.nal(0x01,trailing=not cabac))
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for case in json.loads((BASE/'manifest.json').read_text()):
        original=case['name'];yd,cd=case['luma_depth'],case['chroma_depth'];cabac=case['cabac']
        config=configuration(2,yd,cd,False,cabac,height_mbs=2,max_refs=2)
        raw=(BASE/(original+'.y16')).read_bytes();source=list(struct.unpack('<2048H',raw[:4096]))
        first=(BASE/(original+'-0.packet')).read_bytes();future=future_pcm(source,yd,cd,cabac)
        for changed_list in [0,1]:
            name=f'b-list{changed_list}-'+original
            middle=middle_packet(cabac,case['axis'],case['motion_delta'],case['filter_mode'],changed_list)
            for i,data in enumerate([first,future,middle]):(OUT/f'{name}-{i}.packet').write_bytes(data)
            # The references have equal pixels but distinct identities. Positive
            # motion stays on each constant side, so MC/bi leaves this step
            # unchanged. Either list's MV difference independently forces bS=1.
            for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,middle)],32,32,25)),('y16',raw+raw[:4096])]:(OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(case,name=name,changed_list=changed_list,decode_order=[0,2,1]))
    assert len(cases)==144
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
