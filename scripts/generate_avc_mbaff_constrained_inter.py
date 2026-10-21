"""Owned mono MBAFF constrained intra after inter, with ordinary prediction control."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_mbaff_inter import pcm,packet,expected
from generate_avc_component_fields import configuration as base_config
from generate_avc_pcm_formats import Writer,mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mbaff-constrained-inter'
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for entropy,topology,constrained in itertools.product([-1,0,1,2],['frame','checker'],[False,True]):
        cabac=entropy>=0;cfg=base_config(0,8,8,False,cabac,height_mbs=2,mbaff=True);n=int.from_bytes(cfg[6:8],'big')
        b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2);b.se(0);b.se(0);b.se(0);b.u(1);b.u(int(constrained));b.u(0);b.u(1);b.u(0);b.se(0);pps=b.nal(0x68);cfg=cfg[:8+n]+bytes([1])+len(pps).to_bytes(2,'big')+pps
        source=[[(x*1703+y*7907+x*y*113)&255 for y in range(64)for x in range(32)]]
        target,_=expected(source,[8],topology,(1,-3),'mixed',False,False,constrained)
        wrong,_=expected(source,[8],topology,(1,-3),'mixed',False,False,not constrained);changes=sum(a!=b for a,b in zip(target,wrong));assert changes>0;witness+=changes
        packets=[pcm(source,[8],topology,cabac),packet(topology,cabac,max(entropy,0),(1,-3),'mixed',False,False,components=1)]
        name=f'mono-mbaff-e{entropy}-{topology}-constrained{int(constrained)}'
        for i,payload in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(payload)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(i,i==0,p)for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<4096H',*(source[0]+target)))]:
            (OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,entropy=entropy,constrained=constrained))
    assert len(cases)==16
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps({'constrained_samples':witness},indent=2)+'\n')
if __name__=='__main__':generate()
