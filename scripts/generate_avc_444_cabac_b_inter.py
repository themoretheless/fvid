"""Owned shared 4:4:4 B/direct and biprediction with independently calculated six-tap planes."""
import itertools,json,struct
from pathlib import Path
from generate_avc_444_inter import make_case,configuration,Writer,mux
from generate_avc_444_cabac_inter import pcm,echo
from generate_avc_422_b_motion import future_packet,b_packet
from generate_avc_422_b import WEIGHTS,OFFSETS
from generate_avc_monochrome_fractional import prediction
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-444-cabac-b-inter'
def generate():
    OUT.mkdir(exist_ok=True);cases=[];witness=0
    for (yd,cd),vector,bipred,mode,init_idc in itertools.product([(8,8),(10,12),(14,8)],[(8,-4),(-7,5)],[0,1,2],['temporal','skip','spatial','bi'],[0,1,2]):
        depths=[yd,cd,cd];_,base,_=make_case(yd,cd,(0,0),'skip',False)
        source=[[(x*1703+y*7907+x*y*113+p*349)&((1<<depth)-1)for y in range(16)for x in range(32)]for p,depth in enumerate(depths)]
        future=[prediction(plane,depth,vector)for plane,depth in zip(source,depths)]
        scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i]for i in range(2)]
        vectors=[[0,0],[0,0]]if mode=='spatial'else[scaled,back]
        a=[prediction(plane,depth,vectors[0])for plane,depth in zip(source,depths)]
        z=[prediction(plane,depth,vectors[1])for plane,depth in zip(future,depths)]
        target=[]
        for p,depth in enumerate(depths):
            values=[];maximum=(1<<depth)-1;scale=1<<(depth-8)
            for u,v in zip(a[p],z[p]):
                if bipred==0:value=(u+v+1)//2
                elif bipred==2:value=(43*u+21*v+32)//64
                else:value=((WEIGHTS[0][p]*u+WEIGHTS[1][p]*v+2)>>2)+((sum(row[p]for row in OFFSETS)*scale+1)>>1)
                values.append(max(0,min(maximum,value)))
            target.append(values)
        if mode=='temporal' and bipred==0:
            count=sum(t!=(s+f+1)//2 for ts,ss,fs in zip(target,source,future)for t,s,f in zip(ts,ss,fs));assert count>0;witness+=count
        packets=[pcm(source,depths),future_packet(True,vector,chroma=False,init_idc=init_idc),b_packet(True,bipred,mode,vectors,chroma=True,full_resolution=True,init_idc=init_idc),echo(False,poc=8,init_idc=init_idc)]
        name=f'cabac-b-y{yd}-c{cd}-x{vector[0]}-y{vector[1]}-w{bipred}-{mode}-i{init_idc}'
        cfg=configuration(yd,cd,False,max_refs=2,bipred=bipred,cabac=True)
        for i,p in enumerate(packets):(OUT/f'{name}-{i}.packet').write_bytes(p)
        words=sum(source+future+target+future,[])
        for ext,data in [('avcc',cfg),('mp4',mux(cfg,[(pts,i==0,p)for i,(pts,p)in enumerate(zip([0,2,1,3],packets))],32,16,25)),('y16',struct.pack('<6144H',*words))]:(OUT/f'{name}.{ext}').write_bytes(data)
        cases.append(dict(name=name,yd=yd,cd=cd,mode=mode,bipred=bipred,vector=vector,init_idc=init_idc))
    assert len(cases)==216 and witness>0
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
    (OUT/'witnesses.json').write_text(json.dumps(dict(temporal_motion_samples=witness),indent=2)+'\n')
if __name__=='__main__':generate()
