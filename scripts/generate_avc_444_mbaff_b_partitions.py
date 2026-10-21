"""Owned shared 444 MBAFF B partition tables, per-cell reference parity and weights."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_444_mbaff_inter import configuration,pcm,packet as echo,expected
from generate_avc_444_mbaff_p_partitions import reverse
from generate_avc_422_mbaff_b_partitions import MODES_B,state,coded_packet,reference,motion
from generate_avc_422_mbaff_b_partition_weights import MODES_W
from generate_avc_422_mbaff_b_colocated import SWAP
from generate_avc_422_mbaff_b_field_weights import scale,PAST,CURRENT,FUTURE
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import mux
from avc_fixture_weights import B_PROFILES
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-b-partitions'
@lru_cache(None)
def source_planes(yd,cd,index):
    return [[(x*179+y*911+x*y*13+p*43+index*173)&((1<<d)-1)for y in range(64)for x in range(32)]for p,d in enumerate([yd,cd,cd])]
@lru_cache(None)
def predictions(yd,cd,topology,mode):
    negative=MODES_B.index(mode)%2!=0;cells,_=state(topology,mode,negative);predicted=[]
    for list_index in range(2):
        vectors={key:motion(v[list_index])for key,v in cells.items()};refs={key:reference(v[list_index])or 0 for key,v in cells.items()}
        p,_=expected(source_planes(yd,cd,list_index),[yd,cd,cd],topology,(0,0),'mc',False,False,False,cell_vectors=vectors,cell_references=refs)
        predicted.append(p)
    return cells,predicted
def combine(cells,predicted,depths,topology,weight_mode,wrong_poc=None):
    target=[None]*6144;clipped=0;two_lists=0
    for p,depth in enumerate(depths):
        maximum=(1<<depth)-1
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                address=pair*2+parity;ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16)
                for row in range(16):
                    for col in range(16):
                        values=cells[(address,col//4,row//4)];active=[v is not False for v in values];assert any(active)
                        at=p*2048+(oy+row*(2 if field else 1))*32+ox+col;a,z=predicted[0][at],predicted[1][at];both=all(active);two_lists+=int(both)
                        if weight_mode=='explicit':
                            profile=B_PROFILES['mixed'];denom=profile['denoms'][int(p>0)];wa,oa=profile['entries'][0][p];wz,oz=profile['entries'][1][p]
                            if both:v=((wa*a+wz*z+(1<<denom))>>(denom+1))+((oa+oz)*(1<<(depth-8))+1)//2
                            else:
                                w,o=(wa,oa)if active[0]else(wz,oz);v=((w*(a if active[0]else z)+(1<<(denom-1)if denom else 0))>>denom)+(o<<(depth-8))
                        elif weight_mode!='plain'and both:
                            w1=21
                            if weight_mode=='implicit-field':
                                if field and wrong_poc!='picture':
                                    r0=0 if wrong_poc=='matching'else reference(values[0]);r1=0 if wrong_poc=='matching'else reference(values[1])
                                    w1=scale(CURRENT[parity],PAST[parity^r0],FUTURE[parity^r1])>>2
                                else:w1=42
                            assert -64<=w1<=128;v=((64-w1)*a+w1*z+32)//64
                        else:v=(a+z+1)//2 if both else a if active[0]else z
                        clipped+=int(v<0 or v>maximum);target[at]=max(0,min(maximum,v))
    assert None not in target
    return target,clipped,two_lists
@lru_cache(None)
def samples(yd,cd,topology,mode,weight_mode,wrong_poc=None):
    cells,predicted=predictions(yd,cd,topology,mode)
    return combine(cells,predicted,[yd,cd,cd],topology,weight_mode,wrong_poc)
@lru_cache(None)
def reference_packets(yd,cd,cabac,topology,selected):
    depths=[yd,cd,cd]
    return [pcm(source_planes(yd,cd,0),depths,topology,cabac,bottom_delta=2 if selected else None),pcm(source_planes(yd,cd,1),depths,SWAP[topology],cabac,index=1,poc=6 if selected else 4,bottom_delta=4 if selected else None)]
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={'selected_field_poc':0,'reference_index_poc':0,'clipping':0,'both_lists':0,'explicit_weights':0}
    for(yd,cd),entropy,topology,mode in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,MODES_B):
        for weight_mode in (['plain','explicit','implicit','implicit-field']if mode in MODES_W else['plain']):
            cabac=entropy>=0;init=max(0,entropy);negative=MODES_B.index(mode)%2!=0;aso=negative^cabac;selected=weight_mode=='implicit-field'
            # Nonselected implicit weights need a future POC of 6, rather than 4.
            first,future=reference_packets(yd,cd,cabac,topology,selected)
            if weight_mode=='implicit':future=pcm(source_planes(yd,cd,1),[yd,cd,cd],SWAP[topology],cabac,index=1,poc=6)
            target,clipped,both=samples(yd,cd,topology,mode,weight_mode);witness['clipping']+=clipped;witness['both_lists']+=both
            if selected and topology!='frame'and both:
                for key,wrong in [('selected_field_poc','picture'),('reference_index_poc','matching')]:
                    witness[key]+=sum(a!=b for a,b in zip(target,samples(yd,cd,topology,mode,weight_mode,wrong)[0]))
            if weight_mode=='explicit':witness['explicit_weights']+=sum(a!=b for a,b in zip(target,samples(yd,cd,topology,mode,'plain')[0]))
            cfg=configuration(yd,cd,cabac,False,bipred=0 if weight_mode=='plain'else 1 if weight_mode=='explicit'else 2,bottom_poc=selected)
            packets=[first,future,coded_packet(cabac,topology,mode,negative,False,weights=B_PROFILES['mixed']if weight_mode=='explicit'else None,poc=4 if selected else 2,bottom_delta=4 if selected else None,full_resolution=True,init=init),echo(topology,cabac,init,(0,0),'mc',False,False,index=2,poc=12 if selected else 8 if weight_mode=='implicit'else 6,bottom_delta=2 if selected else None)]
            if aso:packets=[reverse(p)for p in packets]
            name=f'mbaff-bparts-y{yd}-c{cd}-e{entropy}-{topology}-{mode}-{weight_mode}'
            for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
            for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<24576H',*(sum(source_planes(yd,cd,0),[])+sum(source_planes(yd,cd,1),[])+target+sum(source_planes(yd,cd,1),[]))))]:
                (OUT/f'{name}.{ext}').write_bytes(data)
            cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,mode=mode,weight_mode=weight_mode,aso=aso))
    assert len(cases)==3984 and all(v>0 for v in witness.values()),witness
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
