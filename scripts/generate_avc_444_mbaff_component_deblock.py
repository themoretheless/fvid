"""Owned MBAFF 444 component-specific residual activity and mapped QP edges.

ITU-T H.264 (06/2019), 8.5.5, 8.5.8/table 8-15, 8.7.2; no external codec.
"""
import itertools,json,struct
from pathlib import Path
from functools import lru_cache
from generate_avc_444_mbaff_inter import configuration,pcm,packet,expected
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_444_deblock import chroma_qp
from generate_avc_separate_mbaff_8x8_deblock import deblock_mbaff_plane
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-component-deblock'
@lru_cache(None)
def samples(yd,cd,topology,kind,activity,offsets,idc):
    depths=[yd,cd,cd];active=[activity=='all',activity in ['all','cb'],activity in ['all','cr']]
    qps=[26,chroma_qp(26,offsets[0],cd),chroma_qp(26,offsets[1],cd)]
    source=[[(90+20*p+2*((x//4+y//4)%2))<<(d-8)for y in range(64)for x in range(32)]for p,d in enumerate(depths)]
    raw,_=expected(source,depths,topology,(0,0),kind,False,False,False,active=active,qps=qps)
    def filtered(mode,force_activity=False,wrong_qp=False):
        return sum([deblock_mbaff_plane(raw[p*2048:(p+1)*2048],d,topology,mode,False,transform8=kind=='ac8',qp=26 if wrong_qp else qps[p],active=active[p]or force_activity)for p,d in enumerate(depths)],[])
    target=filtered(idc)
    witnesses={
        'filter':sum(a!=b for a,b in zip(raw,target)),
        'component_flags':sum(a!=b for a,b in zip(target,filtered(idc,force_activity=True))),
        'slice_boundary':sum(a!=b for a,b in zip(filtered(0),filtered(2))),
        'filter_qp':sum(a!=b for a,b in zip(target,filtered(idc,wrong_qp=True))),
    }
    wrong,_=expected(source,depths,topology,(0,0),kind,False,False,False,active=active)
    witnesses['residual_qp']=sum(a!=b for a,b in zip(raw,wrong))
    if idc==1:assert target==raw
    assert all(raw[p*2048:(p+1)*2048]==source[p]for p in range(3)if not active[p])
    return source,target,active,witnesses

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={key:0 for key in ['filter','component_flags','slice_boundary','filter_qp','residual_qp']}
    for(yd,cd),entropy,topology,kind,activity,offsets,idc in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,['ac4','ac8'],['all','cb','cr'],[(0,0),(-6,6)],[0,1,2]):
        source,target,active,w=samples(yd,cd,topology,kind,activity,offsets,idc)
        for key,value in w.items():witness[key]+=value
        cabac=entropy>=0;depths=[yd,cd,cd];init=max(0,entropy);cfg=configuration(yd,cd,cabac,False,offsets=offsets)
        packets=[pcm(source,depths,topology,cabac),packet(topology,cabac,init,(0,0),kind,False,False,mode=idc,active=active),packet(topology,cabac,init,(0,0),'mc',False,False,index=2)]
        name=f'mbaff-component-y{yd}-c{cd}-e{entropy}-{topology}-{kind}-{activity}-o{offsets[0]}_{offsets[1]}-d{idc}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<18432H',*(sum(source,[])+target+target)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,kind=kind,activity=activity,offsets=offsets,idc=idc))
    assert len(cases)==1728 and all(v>0 for v in witness.values()),witness
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
