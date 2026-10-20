"""Owned non420 MBAFF PCM pair geometry, mixed modes, ASO, crop and precision."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-component-mbaff-pcm'
MODES={'frame':[False]*4,'field':[True]*4,'columns':[False,True,False,True],'checker':[False,True,True,False]}
def picture(fmt,yd,cd,cabac,topology,aso,index,poc=None,bottom_delta=None,planes_override=None,colour_plane=None):
    planes=[[(x*31+y*73+p*43+index*17)&((1<<depth)-1) for y in range(64) for x in range(w)] if w else [] for p,(w,depth) in enumerate([(32,yd),(16 if fmt==2 else 0,cd),(16 if fmt==2 else 0,cd)])];nals=[]
    if planes_override is not None:
        planes=[list(p) for p in planes_override]
        assert [len(p) for p in planes]==([2048,0,0] if fmt==0 else [2048,1024,1024]) and fmt in [0,2]
        assert all(0<=v<(1<<depth) for p,depth in zip(planes,[yd,cd,cd]) for v in p)
    for pair,field in enumerate(MODES[topology]):
        b=Writer();b.ue(pair);b.ue(2);b.ue(0)
        if colour_plane is not None:b.u(colour_plane,2)
        b.u(index,4);b.u(0)
        if index==0:b.ue(0)
        b.u(index*2 if poc is None else poc,4)
        if bottom_delta is not None:b.se(bottom_delta)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26)
        for parity in range(2):
            if parity==0:
                if cabac:c.decision(70,int(field))
                else:b.u(int(field))
            if cabac:c.decision(3+int(parity==1 and not field),1);b.bits.extend(c.finish());b.align()
            else:b.ue(25);b.align()
            for p,(w,bw,depth) in enumerate([(32,16,yd),(16,8,cd),(16,8,cd)]):
                if fmt==0 and p:continue
                for row in range(16):
                    y=pair//2*32+(row*2+parity if field else row+parity*16)
                    for col in range(bw):b.u(planes[p][y*w+pair%2*bw+col],depth)
            if cabac:c.low=0;c.range=510;c.width=9
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals),sum(planes,[])
def unsupported_cases():
    records=[];accepted=[]
    for fmt,cabac,field in itertools.product([0,2],[False,True],[False,True]):
        topology='field' if field else 'frame';base=f'mbaff-pcm-f{fmt}-y8-c8-e{int(cabac)}-{topology}-a0-crop0';name='compressed-'+base
        b=Writer();b.ue(0);b.ue(2);b.ue(0);b.u(0,4);b.u(0);b.ue(0);b.u(0,4);b.u(0);b.u(0);b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(70,int(field))
        for parity in range(2):
            if not cabac:
                if parity==0:b.u(int(field))
                b.ue(3)
                if fmt==2:b.ue(0)
                b.se(0);b.u(1)
            else:
                c.decision(3+int(parity==1 and not field),1);c.range-=2;c.renormalize()
                for ctx,v in [(6,0),(7,0),(9,1),(10,0)]:c.decision(ctx,v)
                if fmt==2:c.decision(64,0)
                c.decision(60,0);c.decision(86 if parity==1 and not field else 88,0)
        if cabac:b.bits.extend(c.finish())
        nal=b.nal(0x65,trailing=not cabac);original=(OUT/f'{base}-0.packet').read_bytes();length=int.from_bytes(original[:4],'big')
        packet=len(nal).to_bytes(4,'big')+nal+original[4+length:];config=(OUT/f'{base}.avcc').read_bytes()
        for ext,data in [('avcc',config),('packet',packet),('mp4',mux(config,[(0,True,packet)],32,64,25))]:(OUT/f'{name}.{ext}').write_bytes(data)
        n=2048 if fmt==0 else 4096
        raw=(OUT/f'{base}.y16').read_bytes()[:n*2];values=list(struct.unpack('<'+str(n)+'H',raw))
        for y in range(32):
            for x in range(16):values[y*32+x]=128
        if fmt==2:
            for component in range(2):
                for y in range(32):
                    for x in range(8):values[2048+component*1024+y*16+x]=128
        (OUT/f'{name}.y16').write_bytes(struct.pack('<'+str(n)+'H',*values))
        accepted.append(dict(name=name,format=fmt,cabac=cabac,field=field,expect='accepted component compressed MBAFF'))
    (OUT/'unsupported.json').write_text(json.dumps(records,indent=2)+'\n')
    (OUT/'accepted.json').write_text(json.dumps(accepted,indent=2)+'\n')

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for fmt,(yd,cd),cabac,topology,aso,crop in itertools.product([0,2],[(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True]):
        name=f'mbaff-pcm-f{fmt}-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}'
        config=configuration(fmt,yd,cd,crop,cabac,height_mbs=2,mbaff=True);packets=[];samples=[]
        for i in range(2):
            packet,values=picture(fmt,yd,cd,cabac,topology,aso,i);packets.append(packet);samples+=values;(OUT/f'{name}-{i}.packet').write_bytes(packet)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<'+str(len(samples))+'H',*samples))]:(OUT/f'{name}.{ext}').write_bytes(data)
        sx=2 if fmt==2 else 1
        cases.append(dict(name=name,format=fmt,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,crop=[sx,sx,2,2] if crop else [0,0,0,0]))
    assert len(cases)==192
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    unsupported_cases()
if __name__=='__main__':generate()
