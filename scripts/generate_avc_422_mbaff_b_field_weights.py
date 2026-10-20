"""Owned selected-field POC implicit weights and temporal-vector reconstruction."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_b import packet
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_422_mbaff_b_colocated import SWAP,future_samples
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-field-weights'
PAST=[0,2];CURRENT=[4,8];FUTURE=[6,10]
def scale(current,past,future):
    td=max(-128,min(127,future-past));tb=max(-128,min(127,current-past));assert td>0
    tx=(16384+abs(int(td/2)))//td
    return max(-1024,min(1023,(tb*tx+32)>>6))
def orders(field,parity,opposite=False):
    return (CURRENT[parity],PAST[parity^int(opposite)],FUTURE[parity^int(opposite)]) if field else (min(CURRENT),min(PAST),min(FUTURE))
def weighted(a,z,topology,depths,opposite=False,force_frame=False):
    out=a[:];all_weights=[];clipped=0
    for pair,field in enumerate(MODES[topology]):
        for parity in range(2):
            current,past,future=orders(field and not force_frame,parity,opposite)
            w1=scale(current,past,future)>>2;w0=64-w1;assert -64<=w1<=128;all_weights.append([w0,w1])
            for component,depth in enumerate(depths):
                width=32 if component==0 else 16;bw=16 if component==0 else 8;offset=0 if component==0 else 2048+(component-1)*1024
                ox=pair%2*bw;oy=pair//2*32+(parity if field else parity*16)
                for row in range(16):
                    for col in range(bw):
                        at=offset+(oy+row*(2 if field else 1))*width+ox+col;value=(w0*a[at]+w1*z[at]+32)//64
                        out[at]=max(0,min((1<<depth)-1,value));clipped+=int(value!=out[at])
    return out,all_weights,clipped
@lru_cache(None)
def samples(yd,cd,topology,mode):
    reference_topology=SWAP[topology];motion=(8,4);source,later=future_samples(yd,cd,reference_topology,motion);vectors=[]
    for pair,field in enumerate(MODES[topology]):
        col_field=MODES[reference_topology][pair]
        for parity in range(2):
            if mode.startswith('temporal'):
                norm=(motion[0],motion[1]*2 if col_field and not field else int(motion[1]/2) if field and not col_field else motion[1])
                factor=scale(*orders(field,parity));left=tuple((factor*v+128)//256 for v in norm);right=tuple(a-b for a,b in zip(left,norm));vectors.append([left,right])
            elif mode in ['l0','l1','bi-motion']:vectors.append([(1,-1),(-3,1)])
            else:vectors.append([(0,0),(0,0)])
    opposite=mode=='bi-motion'
    a=expected(source,yd,cd,topology,(0,0),opposite,mb_motions=tuple(v[0] for v in vectors));z=expected(later,yd,cd,topology,(0,0),opposite,mb_motions=tuple(v[1] for v in vectors))
    if mode in ['l0','l1']:target=a if mode=='l0' else z;weights=[];clipped=0
    else:
        target,weights,clipped=weighted(a,z,topology,[yd,cd,cd],opposite)
        if topology!='frame':
            assert target!=weighted(a,z,topology,[yd,cd,cd],opposite,force_frame=True)[0],'selected field POCs must change pixels'
            if opposite:assert target!=weighted(a,z,topology,[yd,cd,cd],False)[0],'opposite-reference POCs must change pixels'
    return source,later,target,vectors,weights,clipped

def generate():
    OUT.mkdir(exist_ok=True);cases=[];clipped=0
    for (yd,cd),cabac,topology,mode,crop in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,['l0','l1','bi-motion','spatial','temporal','temporal-skip'],[False,True]):
        reference_topology=SWAP[topology];aso=cabac^crop
        name=f'422-mbaff-field-w-y{yd}-c{cd}-e{int(cabac)}-{topology}-{mode}-crop{int(crop)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True,bipred=2,bottom_poc=True)
        source,later,target,vectors,weights,n=samples(yd,cd,topology,mode);clipped+=n
        first,original=picture(2,yd,cd,cabac,topology,aso,0,bottom_delta=2);assert original==source
        # Explicit modes use the same vector in each half; direct modes derive
        # the potentially different top/bottom vectors from their selected POCs.
        pair_vectors=[vectors[pair*2] for pair in range(4)]
        packets=[first,prediction(cabac,reference_topology,aso,(8,4),False,1,poc=6,bottom_delta=4),packet(cabac,topology,aso,mode,pair_vectors=pair_vectors,poc=4,bottom_delta=4),prediction(cabac,topology,aso,(0,0),False,2,poc=12,bottom_delta=2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,reference_topology=reference_topology,mode=mode,aso=aso,field_pocs=[PAST,CURRENT,FUTURE],mb_vectors=vectors,mb_weights=weights,decode_order=[0,2,1,3],crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==288 and clipped>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('288 own selected-field POC weight movies; clipped predictions:',clipped)
if __name__=='__main__':generate()
