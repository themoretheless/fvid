"""Owned DBBP B bi-merge reduction witnesses, offline."""
import json,struct
import generate_hevc_texture_iv_b as b
import generate_hevc_refinement_b as r
from generate_hevc_illumination_b import pcm,pattern
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_cqt import banks,context,part,merge_index_bins,reference,terminal_zero,texture_amp_bi,OUT
from generate_hevc_dbbp_residual import compose
from hevc_fixture_mp4 import mux

def iv_first_mvd(size,vertical,enabled,cut,amplitudes,shifted=False):
    # PU0's desired motion is zero. Cancel MVP0 inherited from already
    # published target L0 cells (same temporal reference POC in every cell).
    grid={};differences=[]
    def at(x,y):return grid.get((x//4,y//4))if 0<=x<size and 0<=y<size else None
    for cy in range(size//32):
        for cx in range(size//32):
            x=cx*32;y=cy*32;w,h=(16,32)if vertical else(32,16)
            points=[(x-1,y+h),(x-1,y+h-1),(x+w,y-1),(x+w-1,y-1),(x-1,y-1)]
            predicted=next((value for xx,yy in points if (value:=at(xx,yy))is not None),(0,0))
            differences.append(tuple(-v for v in predicted))
            default_sign=-1 if shifted else 1 if (24 if vertical else 16)<cut else -1
            for yy in range(y,y+32,4):
                for xx in range(x,x+32,4):
                    second=xx-x>=16 if vertical else yy-y>=16
                    sign=default_sign if enabled or shifted else 1 if xx-x<cut else -1
                    grid[xx//4,yy//4]=(0,sign*amplitudes[0])if second else(0,0)
    return differences

def entropy(vertical,enabled,explicit_bi=False,size=32,maximum=5,selected_merge=0,first_mvd=None,first_list=0):
    encoder,c=banks(2);ref=context(encoder,153);flag=context(encoder,154)
    greater=context(encoder,198)if first_mvd is not None else None
    count=(size//32)**2
    for index in range(count):
        encoder.decision(c['split0'],False);encoder.decision(c['skip0'],False)
        encoder.decision(c['pred'],False);part(encoder,c,2 if vertical else 1,False,5)
        # PU0 explicitly uses L0[0], zero motion. PU1 selects initial bi-zero merge.
        encoder.decision(c['merge'],False);encoder.decision(c['ip0'],explicit_bi)
        if not explicit_bi:encoder.decision(c['ip4'],first_list==1)
        for _ in range(2 if explicit_bi else 1):
            encoder.decision(ref,False)
            difference=first_mvd[index]if first_mvd is not None else(0,0)
            for value in difference:encoder.decision(c['mvd'],value!=0)
            for value in difference:
                if value:encoder.decision(greater,abs(value)>1)
            for value in difference:
                if abs(value)>1:
                    remaining=abs(value)-2;order=1
                    while remaining>=1<<order:encoder.bypass(True);remaining-=1<<order;order+=1
                    encoder.bypass(False)
                    for shift in reversed(range(order)):encoder.bypass((remaining>>shift)&1)
                if value:encoder.bypass(value<0)
            encoder.decision(c['mvp'],False)
        encoder.decision(c['merge'],True);merge_index_bins(encoder,c,selected_merge,maximum)
        encoder.decision(flag,enabled);encoder.decision(c['root'],False)
        if index+1<count:terminal_zero(encoder)
    return encoder.finish()

def movie(vertical,enabled,gain,weighted=False,explicit_bi=False,size=32,direct=False,iv_cut=None,amplitudes=(4,-8),iv_shift=False,l1_only=False):
    assert not l1_only or iv_cut is not None
    first_list=int(l1_only and size>32)
    raw=control_movie(False,size=size,direct=direct,colored=True,inter_view=iv_cut is not None,dbbp=(vertical,enabled))
    config=raw[raw.index(b'hvcC')+4:];at=23;params=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big');at+=2;nal=config[at:at+length];at+=length
            if weighted and ((int.from_bytes(nal[:2],'big')>>9)&63)==34 and ((int.from_bytes(nal[:2],'big')>>3)&63)==5:
                nal=r.parameter_ids(r.header_pps(5,0,True,constrained=direct),0,[2,2])
            params.append(nal)
    past=pcm(size,False,0,gain);future=pcm(size,False,97,gain)
    own_past=pcm(size,True,0,gain);own_future=pcm(size,True,97,gain)
    depth,_=reference(size,5,False);units=[]
    for poc in [0,2,1]:
        units.append([
            b.header(0,poc,future if poc==2 else texture_amp_bi(size,top=iv_cut,vertical_partition=True,motion_amplitudes=amplitudes,active_lists=(not l1_only,True))if poc==1 and iv_cut is not None else past,0 if poc==1 and iv_cut is not None else 2,negative=[2]if poc==2 else[1]if poc==1 else[],positive=[1]if poc==1 else[]),
            r.header(3,poc,depth,True),
            r.header(5,poc,entropy(vertical,enabled,explicit_bi,size,6 if iv_cut is not None else 5,2 if iv_shift else 0,iv_first_mvd(size,vertical,enabled,iv_cut,(amplitudes[1],amplitudes[0])if first_list else amplitudes,iv_shift)if iv_cut is not None else None,first_list)if poc==1 else own_future if poc==2 else own_past,True,weighted=weighted,merge_extra=int(iv_cut is not None),list_entries=[[0,0,0],None]if l1_only else None)])
    stream=b''.join(b'\0\0\0\1'+nal for nal in params+[nal for au in units for nal in au])
    return mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True,presentation_order=[0,2,1])

def generate():
    cases=[];golden=bytearray();witnesses=0
    configurations=[(v,e,g,w,False,32,False)for v in [False,True]for e in [False,True]for g in [(2,1,-40),(1,2,30)]for w in [False,True]]
    configurations += [(v,False,(2,1,-40),False,True,32,False)for v in [False,True]]
    configurations += [(v,e,(2,1,-40),w,False,128,d)for v in [False,True]for e in [False,True]for w in [False,True]for d in [False,True]]
    for vertical,enabled,gain,weighted,explicit_bi,size,direct in configurations:
        name=f'dbbp-b-'+(f'{size}-{int(direct)}-'if size!=32 else'')+f'{int(vertical)}-{int(enabled)}-{gain[0]}'+('-weighted'if weighted else'')+('-explicit-bi'if explicit_bi else'')+'.mp4'
        (OUT/name).write_bytes(movie(vertical,enabled,gain,weighted,explicit_bi,size,direct))
        pixels=[]
        for c in range(3):
            side=size if c==0 else size//2
            past=[pattern(x,y,c,True,0,gain)for y in range(side)for x in range(side)]
            future=[pattern(x,y,c,True,97,gain)for y in range(side)for x in range(side)]
            if weighted and c==0:
                w0,o0=b.WEIGHTS[0][0];w1,o1=b.WEIGHTS[1][0]
                bi=[min(255,max(0,(w0*a+w1*b+2*(o0+o1+1))//4))for a,b in zip(past,future)]
                past=[min(255,max(0,(w0*a+1)//2+o0))for a in past]
            else:bi=[(a+b+1)//2 for a,b in zip(past,future)]
            output=[0]*(side*side);block=32 if c==0 else 16;scale=32//block
            for by in range(0,side,block):
                for bx in range(0,side,block):
                    get=lambda plane:[plane[(by+y)*side+bx+x]for y in range(block)for x in range(block)]
                    own=get(past);bidir=get(bi)
                    part=compose([bidir if explicit_bi else own,own if enabled else bidir],c,vertical,enabled,(bx*scale,by*scale))
                    if enabled:witnesses+=sum(a!=b for a,b in zip(part,compose([own,bidir],c,vertical,True,(bx*scale,by*scale))))
                    for y in range(block):output[(by+y)*side+bx:(by+y)*side+bx+block]=part[y*block:(y+1)*block]
            pixels.extend(output)
        offset=len(golden)//2;golden.extend(struct.pack('<'+str(len(pixels))+'H',*pixels))
        cases.append(dict(file=name,vertical=vertical,enabled=enabled,gain=gain,weighted=weighted,explicit_bi=explicit_bi,size=size,direct=direct,offset=offset))
    refusal_cases=[]
    for vertical in [False,True]:
        name=f'dbbp-b-{int(vertical)}-1-2-explicit-bi.mp4'
        (OUT/name).write_bytes(movie(vertical,True,(2,1,-40),explicit_bi=True))
        refusal_cases.append(dict(file=name,reason='HEVC DBBP explicit predictor is bidirectional'))
    assert witnesses>0
    (OUT/'dbbp-b.y16').write_bytes(golden)
    (OUT/'dbbp-b.json').write_text(json.dumps(dict(provenance='Owned B slices, PCM reference pictures and DBBP CABAC/MP4; independent uni/bi sample-domain and contour oracle. Offline.',l1_reduction_witnesses=witnesses,cases=cases,refusal_cases=refusal_cases),indent=2)+'\n')
if __name__=='__main__':generate()
