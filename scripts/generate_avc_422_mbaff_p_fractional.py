"""Owned MBAFF 422 signed qpel/eighth-chroma phases with six-tap scalar oracle."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_p import prediction
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p-fractional'
TAPS=[1,-5,20,20,-5,1]
def luma(sample,maximum,x,y,fx,fy):
    clip=lambda n:max(0,min(maximum,n))
    h=lambda a,b:clip((sum(t*sample(a+i-2,b) for i,t in enumerate(TAPS))+16)//32)
    v=lambda a,b:clip((sum(t*sample(a,b+i-2) for i,t in enumerate(TAPS))+16)//32)
    d=lambda a,b:clip((sum(t*s*sample(a+i-2,b+j-2) for i,t in enumerate(TAPS) for j,s in enumerate(TAPS))+512)//1024)
    if fx==fy==0:return sample(x,y)
    if fy==0:return h(x,y) if fx==2 else (h(x,y)+sample(x+int(fx==3),y)+1)//2
    if fx==0:return v(x,y) if fy==2 else (v(x,y)+sample(x,y+int(fy==3))+1)//2
    if fx==fy==2:return d(x,y)
    if fx==2:return (d(x,y)+h(x,y+int(fy==3))+1)//2
    if fy==2:return (d(x,y)+v(x+int(fx==3),y)+1)//2
    return (h(x,y+int(fy==3))+v(x+int(fx==3),y)+1)//2

def expected(source,yd,cd,topology,motion,opposite,wrong_phase=False,nearest=False,pair_motions=None,mb_motions=None,cell_motions=None,cell_references=None):
    result=[]
    for component,depth in enumerate([yd,cd,cd]):
        width=32 if component==0 else 16;bw=16 if component==0 else 8;offset=0 if component==0 else 2048+(component-1)*1024
        dest=[None]*(2048 if component==0 else 1024)
        for pair,field in enumerate(MODES[topology]):
            vector=pair_motions[pair] if pair_motions is not None else motion
            for parity in range(2):
                if mb_motions is not None:vector=mb_motions[pair*2+parity]
                ox=pair%2*bw;logical_y=pair//2*16 if field else pair//2*32+parity*16
                physical_y=pair//2*32+(parity if field else parity*16);selected=parity^int(opposite)
                def sample(x,y):
                    x=max(0,min(width-1,x));y=max(0,min(31 if field else 63,y))
                    return source[offset+(2*y+selected if field else y)*width+x]
                for row in range(16):
                    for col in range(bw):
                        key=(pair*2+parity,col*(1 if component==0 else 2)//4,row//4)
                        if cell_motions is not None:vector=cell_motions[key]
                        if cell_references is not None:selected=parity^(cell_references[key]%2)
                        if component==0:
                            x=ox+col+vector[0]//4;y=logical_y+row+vector[1]//4
                            value=sample(x,y) if nearest else luma(sample,(1<<depth)-1,x,y,vector[0]%4,vector[1]%4)
                        else:
                            qx=(ox+col)*8+vector[0];qy=(logical_y+row)*8+vector[1]*2
                            if wrong_phase and field and opposite:qy+=4 if parity else -4
                            x,fx=divmod(qx,8);y,fy=divmod(qy,8)
                            value=sample(x,y) if nearest else ((8-fx)*(8-fy)*sample(x,y)+fx*(8-fy)*sample(x+1,y)+(8-fx)*fy*sample(x,y+1)+fx*fy*sample(x+1,y+1)+32)//64
                        at=(physical_y+row*(2 if field else 1))*width+ox+col;assert dest[at] is None;dest[at]=value
        assert None not in dest;result+=dest
    return result

def generate():
    OUT.mkdir(exist_ok=True);cases=[];phase_witnesses=0;opposite_witnesses=0
    for (yd,cd),cabac,topology,dx,dy,opposite in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,range(-4,4),range(-2,2),[False,True]):
        if topology=='frame' and opposite:continue
        aso=(dx+dy)%2!=0;crop=dx<0;motion=(dx,dy)
        name=f'422-mbaff-qpel-y{yd}-c{cd}-e{int(cabac)}-{topology}-x{dx}-y{dy}-opp{int(opposite)}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        first,source=picture(2,yd,cd,cabac,topology,aso,0);target=expected(source,yd,cd,topology,motion,opposite)
        if dx%8 or dy%4:
            assert target!=expected(source,yd,cd,topology,motion,opposite,nearest=True);phase_witnesses+=1
        if opposite:
            assert target!=expected(source,yd,cd,topology,motion,False)
            assert target!=expected(source,yd,cd,topology,motion,True,wrong_phase=True);opposite_witnesses+=1
        packets=[first,prediction(cabac,topology,aso,motion,opposite,1),prediction(cabac,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,motion=motion,opposite=opposite,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==1344
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('1344 own fractional 422 MBAFF movies; phase witnesses:',phase_witnesses,'opposite-parity witnesses:',opposite_witnesses)
if __name__=='__main__':generate()
