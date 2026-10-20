"""Owned level-4.1 separate-plane MBAFF intra with independent component layouts."""
import json,struct,itertools
from pathlib import Path
from generate_avc_mbaff_direct_samples import Writer
from generate_avc_separate_intra import lists
from generate_avc_mono_mbaff_intra import picture
from generate_avc_component_mbaff_pcm import MODES
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-mbaff-intra'

def configuration(depth,cabac,scaled):
    b=Writer()
    for v in [244,0,41]:b.u(v,8)
    b.ue(0);b.ue(3);b.u(1);b.ue(depth-8);b.ue(0);b.u(0);b.u(int(scaled))
    if scaled:
        lists(b,[8,16,24,16,16,16])
        for _ in range(6):b.u(0)
    b.ue(0);b.ue(0);b.ue(0);b.ue(2);b.u(0);b.ue(1);b.ue(1);b.u(0);b.u(1);b.u(1);b.u(1)
    for _ in range(4):b.ue(1)
    b.u(0);sps=b.nal(0x67)
    b=Writer();b.ue(0);b.ue(0);b.u(int(cabac));b.u(0);b.ue(0);b.ue(0);b.ue(0);b.u(0);b.u(0,2)
    b.se(0);b.se(0);b.se(0);b.u(1);b.u(0);b.u(0);pps=b.nal(0x68)
    return bytes([1,244,0,41,255,225])+len(sps).to_bytes(2,'big')+sps+bytes([1])+len(pps).to_bytes(2,'big')+pps

def generate():
    OUT.mkdir(exist_ok=True);cases=[];layouts=list(MODES)
    for depth,cabac,base,kind,scaled in itertools.product([8,10,14],[False,True],range(4),['i16-dc','i4-dc','i4-ac'],[False,True]):
        name=f'separate-mbaff-{depth}-e{int(cabac)}-t{base}-{kind}-w{int(scaled)}'
        config=configuration(depth,cabac,scaled);frames=[];golden=[]
        chosen=[layouts[(base+c)%4]for c in range(3)]
        assert len(set(chosen))==3
        for index in range(2):
            packet=bytearray()
            for c,topology in enumerate(chosen):
                part,values=picture(depth,cabac,topology,False,kind,index,colour_plane=c,weight=[8,16,24][c] if scaled else 16)
                packet.extend(part);golden+=values
            packet=bytes(packet);(OUT/f'{name}-{index}.packet').write_bytes(packet);frames.append((index,index==0,packet))
        for suffix,data in [('avcc',config),('mp4',mux(config,frames,32,64,25)),('y16',struct.pack('<12288H',*golden))]:(OUT/f'{name}.{suffix}').write_bytes(data)
        cases.append(dict(name=name,depth=depth,cabac=cabac,layouts=chosen,kind=kind,scaled=scaled))
    assert len(cases)==144
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
