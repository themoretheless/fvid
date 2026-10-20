"""Owned 422 MBAFF reordered explicit/direct/skip B pictures and scalar sampling."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import expected,prediction
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
from avc_fixture_weights import table as weight_table
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b'
def packet(cabac,topology,aso,mode,pair_vectors=None,poc=2,weights=None,bottom_delta=None,colour_plane=None,monochrome=False):
    if monochrome:assert weights is None
    nals=[];direct=not mode.startswith('bi-') and mode not in ['l0','l1'];skip=mode.endswith('-skip');motion=(8,4) if mode=='bi-motion' else (0,0);opposite=mode=='bi-motion'
    code=0 if direct else 1 if mode=='l0' else 2 if mode=='l1' else 3
    active=[0] if code==1 else [1] if code==2 else [0,1]
    for pair,field in enumerate(MODES[topology]):
        vectors=pair_vectors[pair] if pair_vectors is not None else [motion,tuple(-v for v in motion)]
        b=Writer();b.ue(pair);b.ue(1);b.ue(0)
        if colour_plane is not None:b.u(colour_plane,2)
        b.u(2,4);b.u(0);b.u(poc,4)
        if bottom_delta is not None:b.se(bottom_delta)
        b.u(int(not mode.startswith('temporal')));b.u(0);b.u(0);b.u(0)
        if weights is not None:weight_table(b,weights)
        if cabac:b.ue(0)
        b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,26)
        for parity in range(2):
            bottom=parity==1 and not field;skipped=skip and parity==1
            if not cabac:
                b.ue(int(skipped))
                if skipped:continue
                if parity==0:b.u(int(field))
                b.ue(code)
                if field and not direct:
                    for _ in active:b.u(int(not opposite))
                if not direct:
                    for list_index in active:
                        for v in vectors[list_index]:b.se(0 if bottom else v)
                b.ue(0)
            else:
                c.decision(24+int(bottom),int(skipped))
                if skipped:continue
                if parity==0:c.decision(70,int(field))
                mb_type(c,code,int(bottom and not direct))
                if field and not direct:
                    for _ in active:
                        c.decision(54,int(opposite))
                        if opposite:c.decision(58,0)
                if not direct:
                    for list_index in active:
                        for axis,v in enumerate(vectors[list_index]):c.mvd(axis,0 if bottom else v,abs(v) if bottom else 0)
                for ctx in ([75,76,75,76] if bottom else [73,74,75,76]):c.decision(ctx,0)
                if not monochrome:c.decision(77,0)
        if cabac:b.bits.extend(c.finish())
        nal=b.nal(0x01,trailing=not cabac);nals.append(nal)
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,aso,crop,mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True],['bi-zero','bi-motion','spatial','temporal','spatial-skip','temporal-skip']):
        name=f'422-mbaff-b-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-{mode}'
        reference_topology={'frame':'field','field':'frame','columns':'checker','checker':'columns'}[topology]
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        first,source=picture(2,yd,cd,cabac,topology,aso,0)
        future,later=picture(2,yd,cd,cabac,reference_topology,aso,1,poc=4)
        motion=(8,4) if mode=='bi-motion' else (0,0);opposite=mode=='bi-motion'
        a=expected(source,topology,motion,opposite);z=expected(later,topology,tuple(-v for v in motion),opposite)
        target=[(left+right+1)//2 for left,right in zip(a,z)]
        assert target!=a and target!=z,'both references must visibly affect B output'
        if mode=='bi-motion':
            assert target!=[(left+right+1)//2 for left,right in zip(source,later)],'motion must affect B output'
        packets=[first,future,packet(cabac,topology,aso,mode),prediction(cabac,topology,aso,(0,0),False,2,poc=6)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,future),(1,False,packets[2]),(3,False,packets[3])],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,reference_topology=reference_topology,aso=aso,mode=mode,decode_order=[0,2,1,3],crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==576
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('576 own reordered 422 MBAFF B movies')
if __name__=='__main__':generate()
