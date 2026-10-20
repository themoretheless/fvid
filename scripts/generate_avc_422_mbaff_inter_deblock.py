"""Owned 422 MBAFF inter filtering, physical edge traversal and scalar weak filter."""
import itertools,json,struct
from functools import lru_cache
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_422_mbaff_p_partitions import packet,state
from generate_avc_422_mbaff_p_fractional import expected
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-inter-deblock'
@lru_cache(None)
def source_planes(yd,cd):
    return [[(80+x//4*3+y//4*2+p*7+y%2*2)*(1<<(depth-8)) for y in range(64) for x in range(width)] for p,(width,depth) in enumerate([(32,yd),(16,cd),(16,cd)])]
@lru_cache(None)
def predicted(yd,cd,topology,opposite,mode,negative,luma_residual):
    source=sum(source_planes(yd,cd),[]);vectors,_=state(topology,mode,negative)
    target=expected(source,yd,cd,topology,(0,0),opposite,cell_motions=vectors)
    if luma_residual:
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                ox=pair%2*16;oy=pair//2*32+(parity if field else parity*16)
                for row in range(16):
                    for col in range(16):
                        sign=-1 if (pair+parity+col//4+row//4+int(negative))%2 else 1
                        at=(oy+row*(2 if field else 1))*32+ox+col;target[at]=max(0,min((1<<yd)-1,target[at]+sign*52*(1<<(yd-8))))
    return source,target
@lru_cache(None)
def geometry(topology,component):
    width,bw=(32,16) if component==0 else (16,8);owners={};locations=[]
    for pair,field in enumerate(MODES[topology]):
        for parity in range(2):
            ox=pair%2*bw;oy=pair//2*32+(parity if field else parity*16);step=2 if field else 1;address=len(locations);locations.append((ox,oy,step,field))
            for row in range(16):
                for col in range(bw):owners[(ox+col,oy+row*step)]=(address,col,row)
    assert len(owners)==width*64
    return owners,locations

def filtered(source,depth,component,topology,opposite,vectors,luma_residual,filter_mode,wrong_threshold=False,skip_odd=False,skip_second=False):
    out=source[:]
    if filter_mode==1:return out
    width,bw=(32,16) if component==0 else (16,8);owners,locations=geometry(topology,component);maximum=(1<<depth)-1;scale=1<<(depth-8)
    alpha,beta=(255*scale,18*scale) if component==0 else (71*scale,12*scale)
    for address,(ox,oy,row_step,field) in enumerate(locations):
        for vertical in [True,False]:
            for edge in ([0,4,8,12] if vertical and component==0 or not vertical else [0,4]):
                if component and not vertical and skip_odd and edge in [4,12]:continue
                prior=owners.get((ox,oy-1));mixed=not vertical and edge==0 and not field and prior is not None and locations[prior[0]][3]
                for extra in range(1 if skip_second or not mixed else 2):
                    for line in range(16 if vertical else bw):
                        x=ox+(edge if vertical else line);y=oy+(line*row_step if vertical else edge*row_step+extra)
                        step=1 if vertical else width*(2 if field or mixed else 1)
                        q=(x,y);p=(x-1,y) if vertical else (x,y-(2 if field or mixed else 1));previous=owners.get(p)
                        if previous is None:continue
                        owner,px,py=previous
                        if filter_mode==2 and owner//2!=address//2:continue
                        previous_field=locations[owner][3]
                        if luma_residual:strength=2
                        elif field!=previous_field:strength=1
                        else:
                            _,qx,qy=owners[q];subx=1 if component==0 else 2
                            a=vectors[(owner,px*subx//4,py//4)];b=vectors[(address,qx*subx//4,qy//4)]
                            different_reference=field and (owner%2)^int(opposite)!=(address%2)^int(opposite)
                            strength=int(different_reference or abs(a[0]-b[0])>=4 or abs(a[1]-b[1])>=(4 if wrong_threshold or not field else 2))
                        if not strength:continue
                        at=y*width+x;p0,p1,p2=out[at-step],out[at-2*step],out[at-3*step];q0,q1,q2=out[at],out[at+step],out[at+2*step]
                        if abs(p0-q0)>=alpha or abs(p1-p0)>=beta or abs(q1-q0)>=beta:continue
                        tc0=([0,11,15][strength] if component==0 else [0,3,4][strength])*scale
                        ap=abs(p2-p0)<beta;aq=abs(q2-q0)<beta;tc=tc0+(1 if component else int(ap)+int(aq))
                        delta=max(-tc,min(tc,(4*(q0-p0)+p1-q1+4)//8));out[at-step]=max(0,min(maximum,p0+delta));out[at]=max(0,min(maximum,q0-delta))
                        if not component:
                            average=(p0+q0+1)//2
                            if ap:out[at-2*step]=p1+max(-tc0,min(tc0,(p2+average-2*p1)//2))
                            if aq:out[at+step]=q1+max(-tc0,min(tc0,(q2+average-2*q1)//2))
    return out
@lru_cache(None)
def samples(yd,cd,topology,opposite,mode,negative,luma_residual,filter_mode):
    source,pred=predicted(yd,cd,topology,opposite,mode,negative,luma_residual);vectors,_=state(topology,mode,negative);target=[];changes=[];witnesses=[0,0,0]
    for component,depth in enumerate([yd,cd,cd]):
        start=0 if component==0 else 2048+(component-1)*1024;end=start+(2048 if component==0 else 1024);plane=pred[start:end]
        args=(plane,depth,component,topology,opposite,vectors,luma_residual,filter_mode);result=filtered(*args);target+=result;changes.append(sum(a!=b for a,b in zip(plane,result)))
        witnesses[0]+=int(result!=filtered(*args,wrong_threshold=True));witnesses[1]+=int(result!=filtered(*args,skip_odd=True));witnesses[2]+=int(result!=filtered(*args,skip_second=True))
    if filter_mode==1:assert changes==[0,0,0]
    elif luma_residual:assert all(changes),'residual boundaries must visibly filter every component'
    return source,target,changes,witnesses

def generate():
    OUT.mkdir(exist_ok=True);cases=[];witnesses=[0,0,0];changed=0
    for (yd,cd),cabac,topology,opposite,mode,negative,luma_residual,filter_mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],['16x8','mixed','4x4'],[False,True],[False,True],[0,1,2]):
        if topology=='frame' and opposite:continue
        aso=negative^cabac;crop=negative;name=f'422-mbaff-ifilter-y{yd}-c{cd}-e{int(cabac)}-{topology}-opp{int(opposite)}-{mode}-n{int(negative)}-res{int(luma_residual)}-filter{filter_mode}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True);source,target,changes,w=samples(yd,cd,topology,opposite,mode,negative,luma_residual,filter_mode)
        witnesses=[a+b for a,b in zip(witnesses,w)];changed+=int(any(changes));first,original=picture(2,yd,cd,cabac,topology,aso,0,planes_override=source_planes(yd,cd));assert original==source
        packets=[first,packet(cabac,topology,aso,opposite,mode,negative,qp=50,filter_mode=filter_mode,luma_residual=luma_residual),prediction(cabac,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,opposite=opposite,mode=mode,negative=negative,luma_residual=luma_residual,filter_mode=filter_mode,changed_samples=changes,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==1512 and all(witnesses) and changed>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('1512 own MBAFF inter-filter movies; changed cases:',changed,'field/odd-edge/mixed-parity witnesses:',witnesses)
if __name__=='__main__':generate()
