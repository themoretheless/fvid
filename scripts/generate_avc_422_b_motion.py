"""Owned 422 B direct with moving co-located P references and nested scalar MC."""
import json,struct
from pathlib import Path
from generate_avc_pcm_formats import configuration,Writer,mux
from generate_avc_mbaff_direct_samples import CabacWriter
from generate_avc_field_b_cabac_mixed_samples import mb_type
from generate_avc_422_b import pcm,WEIGHTS,OFFSETS
from generate_avc_422_inter import chroma
from generate_avc_monochrome_fractional import prediction
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/avc-422-b-motion'
def mc(planes,depths,vector):return [prediction(planes[0],depths[0],vector),chroma(planes[1],depths[1],vector),chroma(planes[2],depths[2],vector)]
def future_packet(cabac,vector,colour_plane=None,chroma=True):
    b=Writer();b.ue(0);b.ue(0);b.ue(0)
    if colour_plane is not None:b.u(colour_plane,2)
    b.u(1,4);b.u(6,4);b.u(0);b.u(0);b.u(0)
    if cabac:b.ue(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
        for address in range(2):
            c.decision(11+int(address>0),0)
            for ctx in [14,15,16]:c.decision(ctx,0)
            for component in range(2):c.mvd(component,vector[component]if address==0 else 0,abs(vector[component])if address else 0)
            for ctx in ([73,74,75,76]if address==0 else[74,74,76,76]):c.decision(ctx,0)
            if chroma:c.decision(77,0)
            if address==0:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
    else:
        for address in range(2):b.ue(0);b.ue(0);b.se(vector[0]if address==0 else 0);b.se(vector[1]if address==0 else 0);b.ue(0)
    nal=b.nal(0x41,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def b_packet(cabac,bipred,mode,vectors,colour_plane=None,chroma=True):
    b=Writer();b.ue(0);b.ue(1);b.ue(0)
    if colour_plane is not None:b.u(colour_plane,2)
    b.u(2,4);b.u(2,4);b.u(int(mode=='spatial' or mode=='bi'));b.u(0);b.u(0);b.u(0)
    if bipred==1:
        b.ue(1)
        if chroma:b.ue(1)
        for l in range(2):
            b.u(1);b.se(WEIGHTS[l][0]);b.se(OFFSETS[l][0])
            if chroma:
                b.u(1)
                for p in [1,2]:b.se(WEIGHTS[l][p]);b.se(OFFSETS[l][p])
    if cabac:b.ue(0)
    b.se(0);b.ue(1)
    if cabac:
        while len(b.bits)%8:b.u(1)
        c=CabacWriter(0,26)
        for address in range(2):
            c.decision(24+int(address>0 and mode!='skip'),int(mode=='skip'))
            if mode!='skip':
                mb_type(c,3 if mode=='bi'else 0,int(address>0 and mode=='bi'))
                if mode=='bi':
                    for vector in vectors:
                        for component in range(2):c.mvd(component,vector[component]if address==0 else 0,abs(vector[component])if address else 0)
                for ctx in ([73,74,75,76]if address==0 else[74,74,76,76]):c.decision(ctx,0)
                if chroma:c.decision(77,0)
            if address==0:c.range-=2;c.renormalize()
        b.bits.extend(c.finish())
    elif mode=='skip':b.ue(2)
    else:
        for address in range(2):
            b.ue(0);b.ue(3 if mode=='bi'else 0)
            if mode=='bi':
                for vector in vectors:b.se(vector[0]if address==0 else 0);b.se(vector[1]if address==0 else 0)
            b.ue(0)
    nal=b.nal(0x01,trailing=not cabac);return len(nal).to_bytes(4,'big')+nal

def generate():
    OUT.mkdir(exist_ok=True);cases=[]
    for yd,cd in [(8,8),(10,12),(14,8)]:
        depths=[yd,cd,cd]
        a=[[(x*1703+y*7907+x*y*113+p*43)&((1<<depth)-1)for y in range(16)for x in range(w)]for p,(depth,w)in enumerate(zip(depths,[32,16,16]))]
        for vector in [(8,-4),(-7,5)]:
            future=mc(a,depths,vector);scaled=[(85*v+128)//256 for v in vector];back=[scaled[i]-vector[i]for i in range(2)]
            for cabac in [False,True]:
                first=pcm(a,depths,0,cabac);later=future_packet(cabac,vector)
                for bipred in [0,1,2]:
                    config=configuration(2,yd,cd,False,cabac,max_refs=2,bipred=bipred)
                    for mode in ['temporal','skip','spatial','bi']:
                        name=f'422-b-motion-y{yd}-c{cd}-e{int(cabac)}-w{bipred}-x{vector[0]}-y{vector[1]}-{mode}'
                        vectors=[[0,0],[0,0]]if mode=='spatial'else[scaled,back]
                        middle=b_packet(cabac,bipred,mode,vectors)
                        predictions=[mc(a,depths,vectors[0]),mc(future,depths,vectors[1])];target=[]
                        for p,depth in enumerate(depths):
                            maximum=(1<<depth)-1;scale=1<<(depth-8);values=[]
                            for x,y in zip(predictions[0][p],predictions[1][p]):
                                if bipred==0:v=(x+y+1)//2
                                elif bipred==2:v=(43*x+21*y+32)//64
                                else:v=((WEIGHTS[0][p]*x+WEIGHTS[1][p]*y+2)>>2)+((sum(row[p]for row in OFFSETS)*scale+1)>>1)
                                values.append(max(0,min(maximum,v)))
                            target.append(values)
                        if mode=='temporal' and bipred==0:assert sum(target,[])!=[(x+y+1)//2 for x,y in zip(sum(a,[]),sum(future,[]))]
                        for i,data in enumerate([first,later,middle]):(OUT/f'{name}-{i}.packet').write_bytes(data)
                        for suffix,data in [('avcc',config),('mp4',mux(config,[(0,True,first),(2,False,later),(1,False,middle)],32,16,25)),('y16',struct.pack('<3072H',*(sum(a,[])+sum(target,[])+sum(future,[]))))]:(OUT/(name+'.'+suffix)).write_bytes(data)
                        cases.append(dict(name=name,luma_depth=yd,chroma_depth=cd,cabac=cabac,bipred=bipred,mode=mode,colocated_motion=vector,list_motion=vectors,decode_order=[0,2,1]))
    (OUT/'manifest.json').write_text(json.dumps(cases,indent=2)+'\n')
if __name__=='__main__':generate()
