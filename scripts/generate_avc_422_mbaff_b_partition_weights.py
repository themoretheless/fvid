"""Owned weighted B partition combinations and per-cell selected-field POC witnesses."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_b_partitions import MODES_B,SWAP,samples,prediction_samples,coded_packet,reference
from avc_fixture_weights import B_PROFILES
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-b-partition-weights'
MODES_W=['mb8','mb9','mb12','mb13','mb16','mb17','mb20','mb21','sub3','sub8','sub9','sub12','mix0','mix1','mix2','mix3']
def generate():
    OUT.mkdir(exist_ok=True);cases=[];field_witnesses=0;reference_witnesses=0
    for (yd,cd),cabac,topology,mode,weight_mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,MODES_W,['explicit','implicit','implicit-field']):
        mode_id=MODES_B.index(mode);negative=mode_id%2!=0;aso=negative^cabac;crop=weight_mode!='implicit'
        name=f'422-mbaff-bpw-y{yd}-c{cd}-e{int(cabac)}-{topology}-{mode}-{weight_mode}'
        selected=weight_mode=='implicit-field';bipred=1 if weight_mode=='explicit' else 2;weights=B_PROFILES['mixed'] if bipred==1 else None
        source,later,target=samples(yd,cd,topology,mode,negative,weight_mode)
        _,_,cells,_=prediction_samples(yd,cd,topology,mode,negative)
        both=any(all(v is not False for v in value) for value in cells.values())
        if selected and topology!='frame' and both:
            assert target!=samples(yd,cd,topology,mode,negative,'implicit-picture')[2];field_witnesses+=1
            if any(MODES[topology][key[0]//2] and all(v is not False for v in value) and any(reference(v)==1 for v in value) for key,value in cells.items()):
                assert target!=samples(yd,cd,topology,mode,negative,'implicit-matching')[2];reference_witnesses+=1
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True,bipred=bipred,bottom_poc=selected)
        first,a=picture(2,yd,cd,cabac,topology,aso,0,bottom_delta=2 if selected else None)
        future_poc=4 if bipred==1 else 6;future,z=picture(2,yd,cd,cabac,SWAP[topology],aso,1,poc=future_poc,bottom_delta=4 if selected else None);assert source==a and later==z
        packets=[first,future,coded_packet(cabac,topology,mode,negative,aso,weights=weights,poc=4 if selected else 2,bottom_delta=4 if selected else None),prediction(cabac,topology,aso,(0,0),False,2,poc=12 if selected else future_poc+2,bottom_delta=2 if selected else None)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(pts,i==0,p) for i,(pts,p) in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<16384H',*(source+target+later+later)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,mode=mode,weight_mode=weight_mode,negative=negative,aso=aso,decode_order=[0,2,1,3],crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==1152 and field_witnesses>0 and reference_witnesses>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('1152 own weighted B partition movies; selected-field witnesses:',field_witnesses,'reference-index witnesses:',reference_witnesses)
if __name__=='__main__':generate()
