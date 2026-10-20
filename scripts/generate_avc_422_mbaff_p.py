"""Owned 422 MBAFF moving P frames, opposite parity and reference echo."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_pcm_formats import Writer,mux
from generate_avc_422_mbaff_intra import flags
from avc_fixture_weights import table as weight_table
from generate_avc_mbaff_direct_samples import CabacWriter
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p'
def prediction(cabac,topology,aso,motion,opposite,index,dc_sign=0,ac_mode='none',luma_mode='none',poc=None,weights=None,bottom_delta=None,colour_plane=None,monochrome=False):
    if monochrome:assert dc_sign==0 and ac_mode=="none" and weights is None
    chroma_pattern=2 if ac_mode!='none' else int(dc_sign!=0)
    luma_pattern=15 if luma_mode!='none' else 0
    pattern=luma_pattern+16*chroma_pattern
    nals=[]
    for pair,field in enumerate(MODES[topology]):
        b=Writer();b.ue(pair);b.ue(0);b.ue(0)
        if colour_plane is not None:b.u(colour_plane,2)
        b.u(index,4);b.u(0);b.u(index*2 if poc is None else poc,4)
        if bottom_delta is not None:b.se(bottom_delta)
        b.u(0);b.u(0) # default refs, no reordering
        if weights is not None:weight_table(b,weights)
        b.u(0) # no adaptive marking
        if cabac:b.ue(0)
        b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(0,26)
        for parity in range(2):
            bottom=parity==1 and not field
            difference=(0,0) if bottom else motion
            if not cabac:
                b.ue(0)
                if parity==0:b.u(int(field))
                b.ue(0)
                if field:b.u(int(not opposite)) # truncated ue for expanded ref 0/1
                for v in difference:b.se(v)
                b.ue((9 if luma_pattern else 0) if monochrome else ([0,1,6] if not luma_pattern else [11,19,12])[chroma_pattern])
                if pattern:b.se(0)
                if luma_pattern:
                    for block in range(16):
                        bx=(block//4%2)*2+block%2;by=(block//8)*2+block//2%2
                        b.u(1,2);b.u((pair+parity+bx+by+index)%2)
                        if luma_mode=='ac':b.u(3,3)
                        else:b.u(1)
                if chroma_pattern:
                    for component in range(2):
                        sign=dc_sign*(1 if component==0 else -1)*(-1 if (pair+parity+index)%2 else 1)
                        if sign:b.u(1,2);b.u(int(sign<0));b.u(1)
                        else:b.u(1)
                if chroma_pattern==2:
                    for component in range(2):
                        for block,coded in enumerate(flags(component,parity,pair,index,ac_mode)):
                            if coded:b.u(1,2);b.u((component+pair+parity+index+block)%2);b.u(1)
                            else:b.u(1)
            else:
                c.decision(11+int(bottom),0)
                if parity==0:c.decision(70,int(field))
                for ctx in [14,15,16]:c.decision(ctx,0)
                if field:
                    c.decision(54,int(opposite))
                    if opposite:c.decision(58,0)
                for axis,v in enumerate(difference):c.mvd(axis,v,abs(motion[axis]) if bottom else 0)
                for ctx in ([73]*4 if luma_pattern else [75,76,75,76] if bottom else [73,74,75,76]):c.decision(ctx,int(luma_pattern!=0))
                if not monochrome:c.decision(77+2*int(bottom and chroma_pattern>0),int(chroma_pattern>0))
                if chroma_pattern:c.decision(81+2*int(bottom and chroma_pattern==2),int(chroma_pattern==2))
                if pattern:c.decision(60,0)
                if luma_pattern:
                    for block in range(16):
                        bx=(block//4%2)*2+block%2;by=(block//8)*2+block//2%2
                        c.decision(93+int(bx>0)+2*int(by>0 or bottom),1)
                        base=306 if field else 134;last=367 if field else 195
                        if luma_mode=='ac':c.decision(base,0);c.decision(base+1,1);c.decision(last+1,1)
                        else:c.decision(base,1);c.decision(last,1)
                        c.decision(248,0);c.bypass((pair+parity+bx+by+index)%2)
                if chroma_pattern:
                    for component in range(2):
                        sign=dc_sign*(1 if component==0 else -1)*(-1 if (pair+parity+index)%2 else 1)
                        c.decision(97+2*int(bottom and dc_sign!=0),int(sign!=0))
                        if sign:c.decision(321 if field else 149,1);c.decision(382 if field else 210,1);c.decision(258,0);c.bypass(int(sign<0))
                if chroma_pattern==2:
                    for component in range(2):
                        current=flags(component,parity,pair,index,ac_mode);previous=flags(component,0,pair,index,ac_mode)
                        for block,coded in enumerate(current):
                            left=current[block-1] if block%2 else 0
                            top=current[block-2] if block>=2 else previous[6+block] if bottom else 0
                            c.decision(101+left+2*top,coded)
                            if coded:c.decision(324 if field else 152,1);c.decision(385 if field else 213,1);c.decision(267,0);c.bypass((component+pair+parity+index+block)%2)
        if cabac:b.bits.extend(c.finish())
        nal=b.nal(0x41,trailing=not cabac);nals.append(nal)
    if aso:nals.reverse()
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)
def expected(source,topology,motion,opposite,wrong_phase=False):
    out=[]
    for component in range(3):
        width=32 if component==0 else 16;bw=16 if component==0 else 8
        plane=source[:2048] if component==0 else source[2048+(component-1)*1024:3072+(component-1)*1024]
        dest=[None]*len(plane)
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                ox=pair%2*bw;oy=pair//2*32+(parity if field else parity*16)
                for row in range(16):
                    for col in range(bw):
                        x=max(0,min(width-1,ox+col+motion[0]//(4 if component==0 else 8)))
                        y=(pair//2*16+row if field else oy+row)+motion[1]//4
                        if field:
                            y=2*max(0,min(31,y))+(parity^int(opposite))
                        else:y=max(0,min(63,y))
                        at=(oy+row*(2 if field else 1))*width+ox+col
                        value=plane[y*width+x]
                        if wrong_phase and component and opposite:
                            ref_parity=parity^1;direction=-1 if parity==0 else 1
                            other=2*max(0,min(31,(y-ref_parity)//2+direction))+ref_parity
                            value=(value+plane[other*width+x]+1)//2
                        assert dest[at] is None;dest[at]=value
        assert None not in dest;out+=dest
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[];motion_witnesses=0;parity_witnesses=0
    for (yd,cd),cabac,topology,aso,crop,motion,opposite in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True],[(0,0),(8,4),(-8,-4)],[False,True]):
        if topology=='frame' and opposite:continue
        name=f'422-mbaff-p-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-mv{motion[0]}_{motion[1]}-opp{int(opposite)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        first,source=picture(2,yd,cd,cabac,topology,aso,0);target=expected(source,topology,motion,opposite)
        if motion!=(0,0):assert target!=expected(source,topology,(0,0),opposite);motion_witnesses+=1
        if opposite:assert target!=expected(source,topology,motion,False);assert target!=expected(source,topology,motion,True,wrong_phase=True);parity_witnesses+=1
        packets=[first,prediction(cabac,topology,aso,motion,opposite,1),prediction(cabac,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        words=source+target+target
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,motion=motion,opposite=opposite,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==504
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('504 movies; motion witnesses:',motion_witnesses,'opposite-parity witnesses:',parity_witnesses)
if __name__=='__main__':generate()
