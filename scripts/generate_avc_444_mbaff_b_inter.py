"""Owned full-resolution MBAFF B: direct motion conversion and all weight modes.

No external encoder or decoder; full-resolution six-tap scalar sampling.
"""
import itertools,json,struct
from pathlib import Path
from functools import lru_cache
from generate_avc_444_mbaff_inter import configuration,pcm,packet as p_packet,expected
from generate_avc_422_mbaff_b import packet as b_packet
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import mux
from avc_fixture_weights import B_PROFILES
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-b-inter'
SWAP={'frame':'field','field':'frame','columns':'checker','checker':'columns'}
@lru_cache(None)
def samples(yd,cd,topology,mode,weight):
    depths=[yd,cd,cd];motion=(3,3);future_topology=SWAP[topology]
    source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(64)for x in range(32)]for p,depth in enumerate(depths)]
    later,_=expected(source,depths,future_topology,motion,'mc',False,False,False)
    vectors=[]
    for current,reference in zip(MODES[topology],MODES[future_topology]):
        vy=motion[1]*2 if reference and not current else int(motion[1]/2)if current and not reference else motion[1]
        colocated=(motion[0],vy);left=tuple((85*v+128)//256 for v in colocated);right=tuple(a-b for a,b in zip(left,colocated))
        vectors.append([(0,0),(0,0)]if mode.startswith('spatial')else[left,right])
    a,_=expected(source,depths,topology,(0,0),'mc',False,False,False,pair_vectors=[v[0]for v in vectors])
    z,_=expected([later[p*2048:(p+1)*2048]for p in range(3)],depths,topology,(0,0),'mc',False,False,False,pair_vectors=[v[1]for v in vectors])
    target=[]
    for p,depth in enumerate(depths):
        for left,right in zip(a[p*2048:(p+1)*2048],z[p*2048:(p+1)*2048]):
            if weight=='implicit':v=(43*left+21*right+32)//64
            elif weight=='explicit':
                profile=B_PROFILES['mixed'];denom=profile['denoms'][int(p>0)];wa,oa=profile['entries'][0][p];wz,oz=profile['entries'][1][p]
                v=((wa*left+wz*right+(1<<denom))>>(denom+1))+((oa+oz)*(1<<(depth-8))+1)//2
            else:v=(left+right+1)//2
            target.append(max(0,min((1<<depth)-1,v)))
    assert target!=a and target!=z
    return source,later,target,vectors

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for (yd,cd),entropy,topology,mode,weight in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,['bi-derived','temporal','temporal-skip','spatial','spatial-skip'],['plain','explicit','implicit']):
        cabac=entropy>=0;init=max(entropy,0);depths=[yd,cd,cd];source,later,target,vectors=samples(yd,cd,topology,mode,weight)
        if not mode.startswith('spatial'):
            wrong=samples(yd,cd,topology,'spatial',weight)[2];changes=sum(a!=b for a,b in zip(target,wrong));assert changes>0;witness+=changes
        cfg=configuration(yd,cd,cabac,False,bipred={'plain':0,'explicit':1,'implicit':2}[weight])
        packets=[pcm(source,depths,topology,cabac),p_packet(SWAP[topology],cabac,init,(3,3),'mc',False,False,poc=6),b_packet(cabac,topology,False,mode,pair_vectors=vectors if mode=='bi-derived'else None,poc=2,weights=B_PROFILES['mixed']if weight=='explicit'else None,full_resolution=True,init=init),p_packet(topology,cabac,init,(0,0),'mc',False,False,index=2,poc=8)]
        name=f'mbaff-b-y{yd}-c{cd}-e{entropy}-{topology}-{mode}-{weight}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,2,1,3],packets))],32,64,25)),('y16',struct.pack('<24576H',*(sum(source,[])+later+target+later)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,mode=mode,weight=weight))
    assert len(cases)==720 and witness>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps({'temporal_motion_samples':witness},indent=2)+'\n')
if __name__=='__main__':generate()
