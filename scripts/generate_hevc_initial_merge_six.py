"""Owned Annex I five-initial-candidate B merge boundary, offline."""
import json
from generate_hevc_cqt import (exact_texture_motion, banks, context,
    terminal_zero, reference, merge_index_bins, ue, se, packed, escape, OUT)
from hevc_fixture_mp4 import mux

def parameters(direct):
    movie=next(iter(exact_texture_motion(4,active=True,size=64,direct=direct,cqt=False).values()))
    config=movie[movie.index(b'hvcC')+4:]; at=23; units=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big'); at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big'); at+=2
            units.append(config[at:at+length]); at+=length
    return units

def nal(layer,poc,data,kind,temporal=False):
    value='1'+('0'if poc==0 else'')+ue(int(layer>0))+ue(kind)+'1'
    if layer or poc:value+=f'{poc:08b}'
    if poc:
        neg,pos=([1],[1])if kind==0 else([2],[])if poc==3 else([1],[])
        value+='0'+ue(len(neg))+ue(len(pos))
        for distances in [neg,pos]:
            previous=0
            for distance in distances:value+=ue(distance-previous-1)+'1';previous=distance
        value+=ue(0)+ue(0)+str(int(temporal))
    if layer:value+='1' # tex_mc enabled, CQT disabled in SPS
    value+='00'if layer==0 else'0'
    if kind!=2:
        value+='0'
        if kind==0:value+='00'+'0' # list modifications, mvd_l1_zero
        value+='0' # cabac init
        if temporal:value+='1' # collocated_from_l0, sole active reference
        value+=ue(0) # six depth / five texture candidates
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(layer<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def explicit(b,c,difference):
    b.decision(c['mvd'],False);b.decision(c['mvd'],difference!=0)
    if difference:
        b.decision(c['mvd1'],abs(difference)>1)
        if abs(difference)>1:
            remaining=abs(difference)-2;k=1
            while remaining>=1<<k:b.bypass(True);remaining-=1<<k;k+=1
            b.bypass(False)
            for shift in reversed(range(k)):b.bypass((remaining>>shift)&1)
        b.bypass(difference<0)
    b.decision(c['mvp'],False)

def coded(kind,index):
    b,c=banks(1 if kind==1 else 2);c['mvd1']=context(b,198)
    grid={};skips={}
    def at(x,y):return grid.get((x//16,y//16))if 0<=x<64 and 0<=y<64 else None
    for cy in range(2):
        for cx in range(2):
            b.decision(c[f'split{int(cx>0)+int(cy>0)}'],True)
            for dx,dy in [(0,0),(16,0),(0,16),(16,16)]:
                x=cx*32+dx;y=cy*32+dy
                b.decision(c['split0'],False)
                target=kind==0 and (x,y)==(32,32)
                skip_context=int(skips.get((x//16-1,y//16),False))+int(skips.get((x//16,y//16-1),False))
                b.decision(c[f'skip{skip_context}'],target)
                wanted=9 if kind==1 else {(16,32):1,(32,16):2,(48,16):3,(16,48):4}.get((x,y),5)
                if target:
                    merge_index_bins(b,c,index,6)
                    # Four unique spatial candidates, then temporal (9,-9),
                    # then zero. T is absent because the texture source is I.
                    motions=[(1,1),(2,2),(3,3),(4,4),(9,-9),(0,0)]
                    vector=motions[index]
                else:
                    b.decision(c['pred'],False);b.decision(c['part0'],True);b.decision(c['merge'],False)
                    if kind==0:b.decision(c['ip1'],True)
                    for list in range(1 if kind==1 else 2):
                        choices=[at(x-1,y+16),at(x-1,y+15),at(x+16,y-1),at(x+15,y-1),at(x-1,y-1)]
                        predictor=next((v[list]for v in choices if v is not None),(9 if list==0 else -9)if kind==0 else 0)
                        explicit(b,c,wanted-predictor)
                    b.decision(2,False);b.decision(c['root'],False)
                    vector=(wanted,wanted)if kind==0 else(wanted,None)
                grid[x//16,y//16]=vector;skips[x//16,y//16]=target
            if cy*2+cx<3:terminal_zero(b)
    return b.finish()

def generate():
    files={};texture,_=reference(64,5,True,flat=True);depth,_=reference(64,5,False)
    future,_=reference(64,5,False,bias=97)
    for direct in [False,True]:
        for index in [0,1,2,3,4,5]:
            units=[[nal(0,0,texture,2),nal(3,0,depth,2)],
                [nal(0,1,texture,2),nal(3,1,coded(1,0),1)],
                [nal(0,3,texture,2),nal(3,3,future,2)],
                [nal(0,2,texture,2),nal(3,2,coded(0,index),0,True)]]
            stream=b''.join(b'\0\0\0\1'+n for n in parameters(direct)+[n for au in units for n in au])
            name=f'initial-merge-six-{index}'+('-direct'if direct else'')+'.mp4'
            files[name]=mux(stream,8,64,64,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,1,3,2])
    files['initial-merge-six-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC, synthetic 64x64 four-AU two-layer MP4. No private media, external codec, network or test-time generation.',
        specification='H.265 v8 I.8.5.3.2.2: combine only when numCurrMergeCand < 5.',
        decode_pocs=[0,1,3,2],presentation_pocs=[0,1,2,3],
        probe=[32,32,16,16],spatial_order=['A1','B1','B0','A0'],
        initial_motion=[[1,1],[2,2],[3,3],[4,4],[9,-9],[0,0]],
        motion_domain='integer vertical depth samples',
        files=list(files)),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
