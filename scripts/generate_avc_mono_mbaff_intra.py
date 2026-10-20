"""Owned mono MBAFF I16 DC/I4 DC-AC, independent physical prediction oracle."""
import json,struct,itertools
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_pcm_formats import Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-mono-mbaff-intra'
def picture(depth,cabac,topology,aso,kind,index,colour_plane=None,weight=16):
    phase=0 if colour_plane is None else colour_plane
    values=[0]*2048;nals=[];scan_witnesses=0;scale=1<<(depth-8);maximum=(1<<depth)-1
    for pair,field in enumerate(MODES[topology]):
        available=[False]*2048;step=2 if field else 1
        b=Writer();b.ue(pair);b.ue(2);b.ue(0)
        if colour_plane is not None:b.u(colour_plane,2)
        b.u(index,4);b.u(0)
        if index==0:b.ue(0)
        b.u(index*2,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(70,int(field))
        for parity in range(2):
            if not cabac and parity==0:b.u(int(field))
            i16=kind=='i16-dc';sign=-1 if (pair+parity+index+phase)%2==0 else 1
            if cabac:
                c.decision(3+int(i16 and parity==1 and not field),int(i16))
                if i16:
                    c.range-=2;c.renormalize()
                    for ctx,v in [(6,0),(7,0),(9,1),(10,0),(60,0),(88,1)]:c.decision(ctx,v)
                    c.decision(277 if field else 105,1);c.decision(338 if field else 166,1)
                    for ctx,v in [(228,1),(232,1),(232,1),(232,0)]:c.decision(ctx,v)
                    c.bypass(int(sign<0))
                else:
                    for _ in range(16):c.decision(68,1)
                    for _ in range(4):c.decision(73,1)
                    c.decision(60,0)
                    for block in range(16):
                        c.decision(96,1);base=306 if field else 134;last=367 if field else 195
                        if kind=='i4-ac':c.decision(base,0);c.decision(base+1,1);c.decision(last+1,1)
                        else:c.decision(base,1);c.decision(last,1)
                        c.decision(248,0);c.bypass((pair+parity+index+phase+block)%2)
            else:
                b.ue(3 if i16 else 0)
                if not i16:
                    for _ in range(16):b.u(1)
                    b.ue(0) # monochrome intra CBP15
                b.se(0)
                if i16:b.u(5,6);b.u(1,6 if sign<0 else 5);b.u(1)
                else:
                    for block in range(16):b.u(1,2);b.u((pair+parity+index+phase+block)%2);b.u(3,3) if kind=='i4-ac' else b.u(1)
            origin_x=pair%2*16;origin_y=pair//2*32+(parity if field else parity*16)
            blocks=[(0,0,16,sign)] if i16 else [((block&1)*4+((block>>2)&1)*8,((block>>1)&1)*4+(block>>3)*8,4,1 if (pair+parity+index+phase+block)%2==0 else -1) for block in range(16)]
            for bx,by,side,s in blocks:
                x=origin_x+bx;y=origin_y+by*step
                top=[(y-step)*32+x+j for j in range(side)] if y>=step else []
                left=[(y+j*step)*32+x-1 for j in range(side)] if x else []
                edge=[]
                if top and all(available[a] for a in top):edge += [values[a] for a in top]
                if left and all(available[a] for a in left):edge += [values[a] for a in left]
                pred=(sum(edge)+len(edge)//2)//len(edge) if edge else 1<<(depth-1)
                for row in range(side):
                    for col in range(side):
                        residual=(s*13*weight*scale+32)//64 if kind!='i4-ac' else ([2,1,-1,-2][row%4 if field else col%4]*s*8*weight*scale+32)//64
                        at=(y+row*step)*32+x+col;decoded=max(0,min(maximum,pred+residual))
                        if kind=='i4-ac':
                            wrong=([2,1,-1,-2][col%4 if field else row%4]*s*8*weight*scale+32)//64
                            scan_witnesses+=int(decoded!=max(0,min(maximum,pred+wrong)))
                        values[at]=decoded;available[at]=True
        if cabac:b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    if kind=="i4-ac":assert scan_witnesses>0, "AC scan orientation must affect samples"
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals),values

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for depth,cabac,topology,aso,crop,kind in itertools.product([8,10,14],[False,True],MODES,[False,True],[False,True],['i16-dc','i4-dc','i4-ac']):
        name=f'mono-mbaff-y{depth}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-{kind}'
        config=configuration(0,depth,8,crop,cabac,height_mbs=2,mbaff=True);packets=[];words=[]
        for index in range(2):
            packet,values=picture(depth,cabac,topology,aso,kind,index);packets.append(packet);words+=values;(OUT/f'{name}-{index}.packet').write_bytes(packet)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<4096H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=0,luma_depth=depth,chroma_depth=8,cabac=cabac,topology=topology,aso=aso,kind=kind,crop=[1,1,2,2] if crop else [0,0,0,0]))
    assert len(cases)==288
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
