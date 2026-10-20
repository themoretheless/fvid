"""Owned 422 MBAFF chroma loop filtering with physical ownership scalar oracle."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES
from generate_avc_422_mbaff_intra import picture
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-deblock'
def filtered(source,depth,topology,mode,skip_odd=False,frame_strength=False):
    out=source[:]
    if mode==1:return out
    owners=[None]*1024;locations=[]
    # Build a physical address map independently of the decoder's neighbour API.
    for pair,field in enumerate(MODES[topology]):
        for parity in range(2):
            x=pair%2*8;y=pair//2*32+(parity if field else parity*16);step=2 if field else 1
            rows=[y+r*step for r in range(16)];locations.append((x,rows,field))
            for row in rows:
                for col in range(x,x+8):
                    assert owners[row*16+col] is None
                    owners[row*16+col]=len(locations)-1
    assert None not in owners
    scale=1<<(depth-8);maximum=(1<<depth)-1
    for address,(ox,rows,field) in enumerate(locations):
        for vertical in [True,False]:
            for edge in ([0,4] if vertical else [0,4,8,12]):
                if skip_odd and not vertical and edge in [4,12]:continue
                if edge==0 and (ox==0 if vertical else rows[0]<(2 if field else 1)):continue
                mixed=not vertical and edge==0 and not field and locations[owners[(rows[0]-1)*16+ox]][2]
                for extra in range(2 if mixed else 1):
                    for line in range(16 if vertical else 8):
                        x=ox+(edge if vertical else line);y=rows[line] if vertical else rows[edge]+extra
                        step=1 if vertical else 16*(2 if field or mixed else 1);at=y*16+x
                        neighbour=owners[at-step]
                        if mode==2 and neighbour//2!=address//2:continue
                        strength=4 if edge==0 and (vertical or frame_strength or not field and not locations[neighbour][2]) else 3
                        p,p1,q,q1=out[at-step],out[at-2*step],out[at],out[at+step]
                        if abs(p-q)>=15*scale or abs(p1-p)>=6*scale or abs(q1-q)>=6*scale:continue
                        if strength==4:a=(2*p1+p+q1+2)//4;b=(2*q1+q+p1+2)//4
                        else:
                            tc=scale+1;delta=max(-tc,min(tc,(4*(q-p)+p1-q1+4)//8))
                            a=max(0,min(maximum,p+delta));b=max(0,min(maximum,q-delta))
                        out[at-step]=a;out[at]=b
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[];odd_witnesses=0;field_witnesses=0
    for (yd,cd),cabac,topology,aso,crop,mode,ac in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True],[0,1,2],['full','mask']):
        name=f'422-mbaff-filter-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-m{mode}-{ac}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True);packets=[];words=[]
        for index in range(2):
            packet,source=picture(yd,cd,cabac,topology,aso,'i16',1,ac,index,filter_mode=mode);packets.append(packet)
            target=source[:2048]
            for component in range(2):
                plane=source[2048+component*1024:3072+component*1024];result=filtered(plane,cd,topology,mode)
                if mode!=1:assert result!=plane,(name,index,component,'filter must change samples')
                else:assert result==plane
                odd_witnesses+=int(result!=filtered(plane,cd,topology,mode,skip_odd=True))
                field_witnesses+=int(result!=filtered(plane,cd,topology,mode,frame_strength=True))
                target+=result
            words+=target;(OUT/f'{name}-{index}.packet').write_bytes(packet)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<8192H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,filter_mode=mode,ac_mode=ac,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==576
    assert odd_witnesses>0,'all horizontal 422 edge classes need a visible witness'
    assert field_witnesses>0,'field and frame boundary strength must differ visibly'
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    print('576 movies; odd-horizontal witnesses:',odd_witnesses,'field-strength witnesses:',field_witnesses)
if __name__=='__main__':generate()
