"""Own separate-plane I/P/B/P streams with independent moving co-located references."""
import json,struct,itertools
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_422_b_motion import future_packet,b_packet
from generate_avc_monochrome_fractional import prediction
from avc_fixture_mp4 import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-separate-inter'
VECTORS=[(8,-4),(-7,5),(3,6)]

def pcm(source,depth,cabac,plane):
    packet=bytearray()
    for address in range(2):
        b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(plane,2);b.u(0,4);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(3,1);b.bits.extend(c.finish());b.align()
        else:b.ue(25);b.align()
        for y in range(16):
            for x in range(16):b.u(source[y*32+address*16+x],depth)
        if cabac:c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
        nal=b.nal(0x65,trailing=not cabac);packet.extend(len(nal).to_bytes(4,'big')+nal)
    return bytes(packet)

def echo(cabac,plane):
    b=Writer();b.ue(0);b.ue(0);b.ue(0);b.u(plane,2);b.u(2,4);b.u(8,4);b.u(0);b.u(0);b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26);c.decision(11,1);c.range-=2;c.renormalize();c.decision(11,1);b.bits.extend(c.finish())
    else:b.ue(2)
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate():
    OUT.mkdir(exist_ok=True);cases=[];motion_witnesses=0
    for depth,cabac,bipred,mode in itertools.product([8,10,14],[False,True],range(3),['temporal','skip','spatial','bi']):
        name=f'separate-inter-{depth}-e{int(cabac)}-w{bipred}-{mode}'
        config=configuration(3,depth,8,True,cabac,max_refs=2,bipred=bipred,separate=True,level=41)
        packets=[bytearray()for _ in range(4)];pictures=[[]for _ in range(4)]
        maximum=(1<<depth)-1
        for plane,vector in enumerate(VECTORS):
            source=[(x*1703+y*7907+x*y*113+plane*349)&maximum for y in range(16)for x in range(32)]
            future=prediction(source,depth,vector)
            scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i]for i in range(2)]
            vectors=[scaled,back] if mode!='spatial' else [[0,0],[0,0]]
            a=prediction(source,depth,vectors[0]);z=prediction(future,depth,vectors[1])
            if bipred==0:target=[(u+v+1)//2 for u,v in zip(a,z)]
            elif bipred==1:target=[max(0,min(maximum,((3*u-v+2)//4)+(1<<(depth-8))))for u,v in zip(a,z)]
            else:target=[(43*u+21*v+32)//64 for u,v in zip(a,z)]
            if plane:
                wrong=prediction(source,depth,VECTORS[0]);changed=sum(u!=v for u,v in zip(future,wrong))
                assert changed>0;motion_witnesses+=changed
            for frame,packet in enumerate([pcm(source,depth,cabac,plane),future_packet(cabac,vector,colour_plane=plane,chroma=False),b_packet(cabac,bipred,mode,vectors,colour_plane=plane,chroma=False),echo(cabac,plane)]):packets[frame].extend(packet)
            for frame,values in enumerate([source,future,target,future]):pictures[frame]+=values
        for i,packet in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(packet)
        records=[(pts,i==0,bytes(packet)) for i,(pts,packet)in enumerate(zip([0,2,1,3],packets))]
        for suffix,data in [('avcc',config),('mp4',mux(config,records,32,16,25)),('y16',struct.pack('<6144H',*sum(pictures,[])))]: (OUT/f'{name}.{suffix}').write_bytes(data)
        cases.append(dict(name=name,depth=depth,cabac=cabac,bipred=bipred,mode=mode))
    assert len(cases)==72
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    (OUT/'coverage.json').write_text(json.dumps(dict(cases=len(cases),independent_motion_witnesses=motion_witnesses),indent=2)+'\n')
if __name__=='__main__':generate()
