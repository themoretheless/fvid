"""Own separate-plane MBAFF P motion/residual and changed-layout reference echo."""
import itertools,json,struct
from pathlib import Path
from generate_avc_separate_mbaff_intra import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_fractional import luma
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-mbaff-p'
VECTORS=[(3,1),(-7,5),(8,-4)]

def expected(source,depth,topology,motion,opposite,residual,pair_motions=None):
    out=[None]*2048;maximum=(1<<depth)-1;scale=1<<(depth-8)
    for pair,field in enumerate(MODES[topology]):
        vector=pair_motions[pair] if pair_motions is not None else motion
        for parity in range(2):
            ox=pair%2*16;ly=pair//2*(16 if field else 32)+(0 if field else parity*16)
            py=pair//2*32+(parity if field else parity*16);selected=parity^int(opposite)
            def sample(x,y):
                x=max(0,min(31,x));y=max(0,min(31 if field else 63,y))
                return source[(2*y+selected if field else y)*32+x]
            for row in range(16):
                for col in range(16):
                    x=ox+col+vector[0]//4;y=ly+row+vector[1]//4
                    value=luma(sample,maximum,x,y,vector[0]%4,vector[1]%4)
                    if residual!='none':
                        sign=-1 if (pair+parity+col//4+row//4+1)%2 else 1
                        dc=sign*13*16*scale if residual=='dc' else [2,1,-1,-2][(row if field else col)%4]*sign*8*16*scale
                        value=max(0,min(maximum,value+((dc+32)>>6)))
                    at=(py+row*(2 if field else 1))*32+ox+col;assert out[at] is None;out[at]=value
    assert None not in out
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[];layouts=list(MODES);phase_witnesses=0;parity_witnesses=0
    for depth,cabac,base,opposite,residual in itertools.product([8,10,14],[False,True],range(4),[False,True],['none','dc','ac']):
        name=f'separate-mbaff-p-{depth}-e{int(cabac)}-t{base}-o{int(opposite)}-{residual}'
        config=configuration(depth,cabac,False);packets=[bytearray()for _ in range(3)];frames=[[]for _ in range(3)]
        for c,motion in enumerate(VECTORS):
            topology=layouts[(base+c)%4];echo_layout=layouts[(base+c+2)%4]
            source=[(x*1703+y*7907+x*y*113+c*349)&((1<<depth)-1) for y in range(64)for x in range(32)]
            first,raw=picture(0,depth,8,cabac,topology,False,0,planes_override=[source,[],[]],colour_plane=c)
            assert raw==source
            target=expected(source,depth,topology,motion,opposite,residual)
            no_motion=expected(source,depth,topology,(0,0),opposite,residual)
            assert target!=no_motion;phase_witnesses+=sum(a!=b for a,b in zip(target,no_motion))
            if opposite and any(MODES[topology]):
                wrong=expected(source,depth,topology,motion,False,residual)
                assert target!=wrong;parity_witnesses+=sum(a!=b for a,b in zip(target,wrong))
            moving=prediction(cabac,topology,False,motion,opposite,1,luma_mode=residual,colour_plane=c,monochrome=True)
            echo=prediction(cabac,echo_layout,False,(0,0),False,2,colour_plane=c,monochrome=True)
            for i,data in enumerate([first,moving,echo]):packets[i].extend(data)
            for i,data in enumerate([source,target,target]):frames[i]+=data
        for i,packet in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(packet)
        for suffix,data in [('avcc',config),('mp4',mux(config,[(i,i==0,bytes(p))for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<18432H',*sum(frames,[])))]: (OUT/f'{name}.{suffix}').write_bytes(data)
        cases.append(dict(name=name,depth=depth,cabac=cabac,base_layout=base,opposite=opposite,residual=residual))
    assert len(cases)==144
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    (OUT/'coverage.json').write_text(json.dumps(dict(cases=len(cases),motion_witnesses=phase_witnesses,opposite_parity_witnesses=parity_witnesses),indent=2)+'\n')
if __name__=='__main__':generate()
