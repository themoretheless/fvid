"""Owned shared 444 MBAFF temporal-direct subparts with nonzero co-located motion."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_444_mbaff_inter import configuration,pcm,packet,expected
from generate_avc_444_mbaff_b_partitions import source_planes,combine
from generate_avc_444_mbaff_p_partitions import reverse
from generate_avc_422_mbaff_b_partitions import state,coded_packet,reference,motion
from generate_avc_422_mbaff_b_colocated import SWAP
from generate_avc_422_mbaff_b_field_weights import scale,PAST,CURRENT,FUTURE
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import mux
from avc_fixture_weights import B_PROFILES
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-b-temporal-partitions'
@lru_cache(None)
def predicted(yd,cd,topology,mode,negative,selected,zero=False):
    depths=[yd,cd,cd];mv=(-3,-3)if negative else(3,3);source=source_planes(yd,cd,0)
    later,_=expected(source,depths,SWAP[topology],mv,'mc',False,False,False)
    direct={}
    for pair,(field,colocated_field)in enumerate(zip(MODES[topology],MODES[SWAP[topology]])):
        norm=(mv[0],mv[1]*2 if colocated_field and not field else int(mv[1]/2)if field and not colocated_field else mv[1])
        for parity in range(2):
            factor=scale(CURRENT[parity],PAST[parity],FUTURE[parity])if selected and field else scale(4,0,6)if selected else scale(2,0,6)
            left=tuple((factor*v+128)//256 for v in norm);right=tuple(a-b for a,b in zip(left,norm))
            direct[pair*2+parity]=[(0,(0,0)if zero else left),(0,(0,0)if zero else right)]
    cells,_=state(topology,mode,negative,direct_by_address=direct);pred=[]
    for li,planes in enumerate([source,[later[p*2048:(p+1)*2048]for p in range(3)]]):
        vectors={key:motion(v[li])for key,v in cells.items()};refs={key:reference(v[li])or 0 for key,v in cells.items()}
        values,_=expected(planes,depths,topology,(0,0),'mc',False,False,False,cell_vectors=vectors,cell_references=refs);pred.append(values)
    return later,cells,pred
@lru_cache(None)
def samples(yd,cd,topology,mode,negative,weight_mode,zero=False,wrong_poc=None):
    later,cells,pred=predicted(yd,cd,topology,mode,negative,weight_mode=='implicit-field',zero)
    target,clipped,_=combine(cells,pred,[yd,cd,cd],topology,weight_mode,wrong_poc)
    return later,target,clipped

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={'temporal_motion':0,'selected_field_poc':0,'reference_index_poc':0,'clipping':0}
    for(yd,cd),entropy,topology,mode,negative,weight_mode in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,['sub0','mix0','mix3'],[False,True],['plain','explicit','implicit','implicit-field']):
        cabac=entropy>=0;init=max(entropy,0);selected=weight_mode=='implicit-field';mv=(-3,-3)if negative else(3,3)
        later,target,clipped=samples(yd,cd,topology,mode,negative,weight_mode);witness['clipping']+=clipped
        changes=sum(a!=b for a,b in zip(target,samples(yd,cd,topology,mode,negative,weight_mode,True)[1]));assert changes>0;witness['temporal_motion']+=changes
        if selected and topology!='frame':
            for key,wrong in [('selected_field_poc','picture'),('reference_index_poc','matching')]:witness[key]+=sum(a!=b for a,b in zip(target,samples(yd,cd,topology,mode,negative,weight_mode,wrong_poc=wrong)[1]))
        cfg=configuration(yd,cd,cabac,False,bipred=0 if weight_mode=='plain'else 1 if weight_mode=='explicit'else 2,bottom_poc=selected)
        source=source_planes(yd,cd,0);packets=[pcm(source,[yd,cd,cd],topology,cabac,bottom_delta=2 if selected else None),packet(SWAP[topology],cabac,init,mv,'mc',False,False,poc=6,bottom_delta=4 if selected else None),coded_packet(cabac,topology,mode,negative,False,weights=B_PROFILES['mixed']if weight_mode=='explicit'else None,poc=4 if selected else 2,bottom_delta=4 if selected else None,full_resolution=True,init=init,spatial=False),packet(topology,cabac,init,(0,0),'mc',False,False,index=2,poc=12 if selected else 8,bottom_delta=2 if selected else None)]
        aso=negative^cabac
        if aso:packets=[reverse(p)for p in packets]
        name=f'mbaff-temporal-y{yd}-c{cd}-e{entropy}-{topology}-{mode}-n{int(negative)}-{weight_mode}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<24576H',*(sum(source,[])+later+target+later)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,mode=mode,negative=negative,weight_mode=weight_mode,aso=aso))
    assert len(cases)==1152 and all(v>0 for v in witness.values()),witness
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
