"""Owned shared 444 B fields with selected-field POCs, moving co-located fields and full-resolution chroma."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_field_inter import configuration,pcm,p_field
from generate_avc_mono_field_b import b_field
from generate_avc_monochrome_fractional import prediction
from generate_avc_pcm_formats import mux
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-field-b-inter'
W=[[3,1,-2],[-1,3,1]];O=[[-5,7,64],[90,-4,-32]]
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for(yd,cd),entropy,reverse,weight,mode,vector in itertools.product([(8,8),(10,12),(14,8)],[-1,0,1,2],[False,True],[0,1,2],['l0','l1','bi','spatial','temporal','skip'],[(8,-4),(-7,5)]):
        depths=[yd,cd,cd];cabac=entropy>=0;init=max(0,entropy);order=[True,False]if reverse else[False,True]
        source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(32)for x in range(32)]for p,depth in enumerate(depths)]
        scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i]for i in range(2)];vectors=[[0,0],[0,0]]if mode=='spatial'else[scaled,back]
        futures={};targets={}
        for bottom in [False,True]:
            future=[];target=[]
            for p,depth in enumerate(depths):
                compact=[v for y in range(16)for v in source[p][(y*2+int(bottom))*32:(y*2+int(bottom)+1)*32]]
                predicted=prediction(compact,depth,vector);future.append(predicted)
                a=prediction(compact,depth,vectors[0]);z=prediction(predicted,depth,vectors[1]);values=[];scale=1<<(depth-8);maximum=(1<<depth)-1;denom=1 if p==0 else 2
                for u,v in zip(a,z):
                    if mode in ['l0','l1']:
                        l=0 if mode=='l0'else 1;value=u if l==0 else v
                        if weight==1:value=((W[l][p]*value+(1<<(denom-1)))>>denom)+O[l][p]*scale
                    elif weight==0:value=(u+v+1)//2
                    elif weight==2:value=(43*u+21*v+32)//64
                    else:value=((W[0][p]*u+W[1][p]*v+(1<<denom))>>(denom+1))+(((O[0][p]+O[1][p])*scale+1)>>1)
                    values.append(max(0,min(maximum,value)))
                if mode=='temporal'and weight==0:
                    count=sum(v!=(u+z+1)//2 for v,u,z in zip(values,compact,predicted));assert count>0;witness+=count
                target.append(values)
            futures[bottom]=future;targets[bottom]=target
        packets=[pcm(i,bottom,source,depths,cabac)for i,bottom in enumerate(order)]
        packets += [p_field(i,bottom,cabac,vector,'mc',False,False,init,poc_base=6)for i,bottom in enumerate(order)]
        packets += [b_field(bottom,i,cabac,mode,weight,vectors,chroma=True,full_resolution=True,init_idc=init)for i,bottom in enumerate(order)]
        def weave(fields):return [v for p in range(3)for y in range(16)for bottom in [False,True]for v in fields[bottom][p][y*32:(y+1)*32]]
        expected=sum(source,[])+weave(futures)+weave(targets)
        cfg=configuration(yd,cd,cabac,False,bipred=weight)
        name=f'field-b-y{yd}-c{cd}-e{entropy}-r{int(reverse)}-w{weight}-{mode}-x{vector[0]}-y{vector[1]}'
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,1,4,5,2,3],packets))],32,32,50)),('y16',struct.pack('<9216H',*expected))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,entropy=entropy,reverse=reverse,weight=weight,mode=mode,vector=vector))
    assert len(cases)==864 and witness>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n');(OUT/'witnesses.json').write_text(json.dumps(dict(temporal_motion_samples=witness),indent=2)+'\n')
if __name__=='__main__':generate()
