"""Owned full-resolution MBAFF P partitions, per-cell qpel and ASO."""
import itertools,json,struct
from pathlib import Path
from functools import lru_cache
from generate_avc_444_mbaff_inter import configuration,pcm,packet as echo,expected
from generate_avc_422_mbaff_p_partitions import packet,state,MODES_P
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-p-partitions'
def reverse(packet):
    nals=[];at=0
    while at<len(packet):
        size=int.from_bytes(packet[at:at+4],'big');assert size and at+4+size<=len(packet)
        nals.append(packet[at:at+4+size]);at+=4+size
    return b''.join(reversed(nals))
@lru_cache(None)
def samples(yd,cd,topology,opposite,mode,negative):
    depths=[yd,cd,cd];source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(64)for x in range(32)]for p,depth in enumerate(depths)]
    vectors,_=state(topology,mode,negative)
    target,_=expected(source,depths,topology,(0,0),'mc',False,opposite,False,cell_vectors=vectors)
    wrong,_=expected(source,depths,topology,vectors[(0,0,0)],'mc',False,opposite,False)
    variation=sum(a!=b for a,b in zip(target,wrong));assert variation>0
    return source,target,variation

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for(yd,cd),entropy,topology,opposite,mode,negative in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,[False,True],MODES_P,[False,True]):
        if topology=='frame'and opposite:continue
        if mode=='ref0-8x8'and(entropy>=0 or opposite):continue
        cabac=entropy>=0;init=max(entropy,0);aso=negative^cabac;source,target,variation=samples(yd,cd,topology,opposite,mode,negative);witness+=variation
        cfg=configuration(yd,cd,cabac,False)
        packets=[pcm(source,[yd,cd,cd],topology,cabac),packet(cabac,topology,False,opposite,mode,negative,full_resolution=True,init=init),echo(topology,cabac,init,(0,0),'mc',False,False,index=2)]
        if aso:packets=[reverse(p)for p in packets]
        name=f'mbaff-parts-y{yd}-c{cd}-e{entropy}-{topology}-o{int(opposite)}-{mode}-n{int(negative)}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<18432H',*(sum(source,[])+target+target)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,opposite=opposite,mode=mode,negative=negative,aso=aso))
    assert len(cases)==1200 and witness>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps({'partition_motion_samples':witness},indent=2)+'\n')
if __name__=='__main__':generate()
