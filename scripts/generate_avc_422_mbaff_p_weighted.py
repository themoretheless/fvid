"""Owned 422 MBAFF component weights, clipping, residual and weighted echo."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_422_mbaff_p_residual import residual
from generate_avc_pcm_formats import mux
from avc_fixture_weights import PROFILES,uni
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p-weighted'
def generate():
    OUT.mkdir(exist_ok=True);cases=[];clipped=0
    for (yd,cd),cabac,topology,opposite,motion,profile,residuals in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[(1,-1),(-3,1)],PROFILES,[False,True]):
        if topology=='frame' and opposite:continue
        aso=motion[0]<0;crop=residuals;weights=PROFILES[profile];dc_sign=-1 if motion[0]<0 else 1;luma_mode='dc' if profile=='mixed' else 'ac'
        name=f'422-mbaff-wp-y{yd}-c{cd}-e{int(cabac)}-{topology}-opp{int(opposite)}-mv{motion[0]}_{motion[1]}-{profile}-res{int(residuals)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True,weighted=True);first,source=picture(2,yd,cd,cabac,topology,aso,0)
        pred=expected(source,yd,cd,topology,motion,opposite);target,n=uni(pred,[yd,cd,cd],weights);clipped+=n
        if profile=='default':assert target==pred
        else:assert target!=pred
        if residuals:target=residual(target,yd,cd,topology,dc_sign,'mask',luma_mode)
        packets=[first,prediction(cabac,topology,aso,motion,opposite,1,dc_sign if residuals else 0,'mask' if residuals else 'none',luma_mode if residuals else 'none',weights=weights),prediction(cabac,topology,aso,(0,0),False,2,weights=PROFILES['default'])]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,opposite=opposite,motion=motion,profile=profile,weights=weights,residuals=residuals,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==504 and clipped>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('504 own weighted P MBAFF movies; clipped predictions:',clipped)
if __name__=='__main__':generate()
