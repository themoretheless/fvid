"""Owned full-resolution MBAFF residual deblocking, independent physical edge traversal."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_mbaff_inter import configuration,pcm,packet,expected
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_separate_mbaff_8x8_deblock import deblock_mbaff_plane
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-mbaff-inter-deblock'
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness={'filter_samples':0,'slice_boundary_samples':0}
    for (yd,cd),entropy,topology,kind,idc in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],MODES,['ac4','ac8'],[0,2]):
        cabac=entropy>=0;depths=[yd,cd,cd];source=[[(96+x%4+y%4)<<(d-8)for y in range(64)for x in range(32)]for d in depths]
        unfiltered,_=expected(source,depths,topology,(0,0),kind,False,False,False)
        def filtered(mode):return sum([deblock_mbaff_plane(unfiltered[p*2048:(p+1)*2048],d,topology,mode,False,transform8=kind=='ac8')for p,d in enumerate(depths)],[])
        target=filtered(idc);changes=sum(a!=b for a,b in zip(target,unfiltered));assert changes>0;witness['filter_samples']+=changes
        witness['slice_boundary_samples']+=sum(a!=b for a,b in zip(filtered(0),filtered(2)))
        cfg=configuration(yd,cd,cabac,False);packets=[pcm(source,depths,topology,cabac),packet(topology,cabac,max(0,entropy),(0,0),kind,False,False,mode=idc),packet(topology,cabac,max(0,entropy),(0,0),'mc',False,False,index=2)]
        name=f'mbaff-filter-y{yd}-c{cd}-e{entropy}-{topology}-{kind}-d{idc}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<18432H',*(sum(source,[])+target+target)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,topology=topology,kind=kind,idc=idc))
    assert len(cases)==192 and all(v>0 for v in witness.values())
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(witness,indent=2)+'\n')
if __name__=='__main__':generate()
