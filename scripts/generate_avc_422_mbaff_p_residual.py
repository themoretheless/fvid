"""Owned moving 422 MBAFF P luma/chroma DC/AC with independent scalar bases."""
import itertools,json,struct
from pathlib import Path
from generate_avc_component_fields import configuration
from generate_avc_component_mbaff_pcm import MODES,picture
from generate_avc_422_mbaff_intra import flags
from generate_avc_422_mbaff_p import prediction,expected
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-mbaff-p-residual'
def residual(source,yd,cd,topology,dc_sign,ac_mode,luma_mode):
    out=source[:];witnesses=[0,0,0];index=1
    for component,depth in enumerate([yd,cd,cd]):
        width=32 if component==0 else 16;bw=16 if component==0 else 8
        offset=0 if component==0 else 2048+(component-1)*1024
        scale=1<<(depth-8);maximum=(1<<depth)-1
        for pair,field in enumerate(MODES[topology]):
            for parity in range(2):
                ox=pair%2*bw;oy=pair//2*32+(parity if field else parity*16)
                sign=dc_sign*(1 if component==1 else -1)*(-1 if (pair+parity+index)%2 else 1)
                qpdc=29+6*(depth-8);value=sign*18*16
                dc=value<<(qpdc//6-6) if qpdc//6>=6 else (value+(1<<(5-qpdc//6)))>>(6-qpdc//6)
                for row in range(16):
                    for col in range(bw):
                        block=row//4*(bw//4)+col//4
                        if component==0:
                            s=-1 if (pair+parity+col//4+row//4+index)%2 else 1
                            correct=other=s*13*16*scale if luma_mode=='dc' else 0
                            if luma_mode=='ac':
                                correct=[2,1,-1,-2][row%4 if field else col%4]*s*8*16*scale
                                other=[2,1,-1,-2][col%4 if field else row%4]*s*8*16*scale
                        else:
                            s=-1 if (component-1+pair+parity+index+block)%2 else 1
                            ac=s*8*16*scale if flags(component-1,parity,pair,index,ac_mode)[block] else 0
                            correct=dc+[2,1,-1,-2][row%4 if field else col%4]*ac
                            other=dc+[2,1,-1,-2][col%4 if field else row%4]*ac
                        at=offset+(oy+row*(2 if field else 1))*width+ox+col
                        a=max(0,min(maximum,source[at]+(correct+32)//64));b=max(0,min(maximum,source[at]+(other+32)//64))
                        out[at]=a;witnesses[component]+=int(a!=b)
        start=offset;end=offset+(2048 if component==0 else 1024)
        if component==0 or dc_sign!=0 or ac_mode!='none':assert out[start:end]!=source[start:end]
        if (component==0 and luma_mode=='ac') or (component and ac_mode!='none'):assert witnesses[component]>0,'scan orientation must change clipped output'
    return out

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for (yd,cd),cabac,topology,aso,crop,dc_sign,ac_mode,luma_mode in itertools.product([(8,8),(10,12),(14,8)],[False,True],MODES,[False,True],[False,True],[-1,0,1],['none','full','mask'],['dc','ac']):
        motion=(8,4) if dc_sign>=0 else (-8,-4);opposite=topology!='frame' and dc_sign!=0
        name=f'422-mbaff-p-res-y{yd}-c{cd}-e{int(cabac)}-{topology}-a{int(aso)}-crop{int(crop)}-dc{dc_sign}-{ac_mode}-l{luma_mode}'
        config=configuration(2,yd,cd,crop,cabac,height_mbs=2,mbaff=True)
        first,source=picture(2,yd,cd,cabac,topology,aso,0);pred=expected(source,topology,motion,opposite);target=residual(pred,yd,cd,topology,dc_sign,ac_mode,luma_mode)
        packets=[first,prediction(cabac,topology,aso,motion,opposite,1,dc_sign,ac_mode,luma_mode),prediction(cabac,topology,aso,(0,0),False,2)]
        for i,data in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(data)
        for ext,data in [('avcc',config),('mp4',mux(config,[(i,i==0,p) for i,p in enumerate(packets)],32,64,25)),('y16',struct.pack('<12288H',*(source+target+target)))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,format=2,luma_depth=yd,chroma_depth=cd,cabac=cabac,topology=topology,aso=aso,motion=motion,opposite=opposite,dc_sign=dc_sign,ac_mode=ac_mode,luma_mode=luma_mode,crop=[2,2,2,2] if crop else [0,0,0,0]))
    assert len(cases)==1728
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');print('1728 own 422 MBAFF P residual/echo movies')
if __name__=='__main__':generate()
