"""Owned 422 reordered B L0/L1/bi/spatial/temporal/skip and weight witnesses."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-b'
WEIGHTS=[[3,1,-2],[-1,3,4]];OFFSETS=[[-7,5,64],[9,-4,-32]]
def pcm(planes,depths,index,cabac):
    nals=[]
    for address in (range(2)if cabac else[0]):
        b=Writer();b.ue(address);b.ue(2);b.ue(0);b.u(index,4)
        if index==0:b.ue(0)
        b.u(index*6,4)
        if index==0:b.u(0);b.u(0)
        else:b.u(0)
        b.se(0);b.ue(1)
        if cabac:
            while len(b.bits)%8:b.u(1)
            c=CabacWriter(-1,26);c.decision(3,1);b.bits.extend(c.finish());b.align()
            addresses=[address]
        else:addresses=range(2)
        for mb in addresses:
            if not cabac:b.ue(25);b.align()
            for p,(depth,w,bw)in enumerate(zip(depths,[32,16,16],[16,8,8])):
                for y in range(16):
                    for x in range(bw):b.u(planes[p][y*w+mb*bw+x],depth)
        if cabac:c.low=0;c.range=510;c.width=9;b.bits.extend(c.finish())
        nals.append(b.nal(0x65 if index==0 else 0x41,trailing=not cabac))
    return b''.join(len(n).to_bytes(4,'big')+n for n in nals)

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        depths=[yd,cd,cd]
        a=[[(x*1703+y*7907+x*y*113+p*43)&((1<<depth)-1)for y in range(16)for x in range(w)]for p,(depth,w)in enumerate(zip(depths,[32,16,16]))]
        future=[[(3*v+29)&((1<<depth)-1)for v in plane]for plane,depth in zip(a,depths)]
        assert all(x!=y for x,y in zip(a,future))
        for cabac in [False,True]:
            first=pcm(a,depths,0,cabac);later=pcm(future,depths,1,cabac)
            for bipred in [0,1,2]:
                config=configuration(2,yd,cd,False,cabac,max_refs=2,bipred=bipred)
                for mode in ['l0','l1','bi','spatial','temporal','skip']:
                    name=f'422-b-y{yd}-c{cd}-e{int(cabac)}-w{bipred}-{mode}'
                    b=Writer();b.ue(0);b.ue(1);b.ue(0);b.u(2,4);b.u(2,4);b.u(int(mode!='temporal'));b.u(0);b.u(0);b.u(0)
                    if bipred==1:
                        b.ue(1);b.ue(1)
                        for l in range(2):
                            b.u(1);b.se(WEIGHTS[l][0]);b.se(OFFSETS[l][0]);b.u(1)
                            for p in [1,2]:b.se(WEIGHTS[l][p]);b.se(OFFSETS[l][p])
                    if cabac:b.ue(0)
                    b.se(0);b.ue(1)
                    code={'l0':1,'l1':2,'bi':3,'spatial':0,'temporal':0,'skip':0}[mode]
                    if cabac:
                        while len(b.bits)%8:b.u(1)
                        c=CabacWriter(0,26)
                        for address in range(2):
                            c.decision(24+int(address==1 and mode!='skip'),int(mode=='skip'))
                            if mode!='skip':
                                mb_type(c,code,int(address==1 and code!=0))
                                for _ in range(2 if code==3 else 1 if code in [1,2]else 0):c.mvd(0,0);c.mvd(1,0)
                                for ctx in ([73,74,75,76]if address==0 else[74,74,76,76]):c.decision(ctx,0)
                                c.decision(77,0)
                            if address==0:c.range-=2;c.renormalize()
                        b.bits.extend(c.finish())
                    elif mode=='skip':b.ue(2)
                    else:
                        for _ in range(2):
                            b.ue(0);b.ue(code)
                            for _ in range(2 if code==3 else 1 if code in [1,2]else 0):b.se(0);b.se(0)
                            b.ue(0)
                    nal=b.nal(0x01,trailing=not cabac);middle=len(nal).to_bytes(4,'big')+nal;target=[]
                    for p,depth in enumerate(depths):
                        maximum=(1<<depth)-1;scale=1<<(depth-8);values=[]
                        for x,y in zip(a[p],future[p]):
                            if mode in ['l0','l1']:
                                l=int(mode=='l1');v=[x,y][l]
                                v=((WEIGHTS[l][p]*v+1)>>1)+OFFSETS[l][p]*scale if bipred==1 else v
                            elif bipred==0:v=(x+y+1)//2
                            elif bipred==2:v=(43*x+21*y+32)//64 # POCs0/6, current2
                            else:v=((WEIGHTS[0][p]*x+WEIGHTS[1][p]*y+2)>>2)+((sum(row[p]for row in OFFSETS)*scale+1)>>1)
                            values.append(max(0,min(maximum,v)))
                        target.append(values)
                    for i,data in enumerate([first,later,middle]):(OUT/f'{name}-{i}.packet').write_bytes(data)
                    for suffix,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,later),(1,False,middle)],32,16,25)),('y16',struct.pack('<3072H',*(sum(a,[])+sum(target,[])+sum(future,[]))))]:(OUT/(name+'.'+suffix)).write_bytes(data)
                    cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bipred=bipred,mode=mode,decode_order=[0,2,1]))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
