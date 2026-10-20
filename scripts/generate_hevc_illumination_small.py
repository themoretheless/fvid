"""Owned 8x8 IC CUs: chroma follows ordinary prediction per I.8.5.3.3.1 step 4."""
import json,struct
from generate_hevc_texture_depth_refinement import control_movie
from generate_hevc_illumination import parameters
from generate_hevc_cqt import banks,context,merge_index_bins,OUT
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_layered_sps import ue
from generate_hevc_layered_pps import se
from generate_he_aac_packet_fixtures import boxes
from hevc_fixture_mp4 import mux

def generate():
    raw=control_movie(False,colored=True,illumination=True)
    config=raw[raw.index(b'hvcC')+4:];at=23;params=[]
    for _ in range(config[22]):
        count=int.from_bytes(config[at+1:at+3],'big');at+=3
        for _ in range(count):
            length=int.from_bytes(config[at:at+2],'big');at+=2;params.append(config[at:at+length]);at+=length
    mdat=next(p for k,p in boxes(raw)if k==b'mdat');at=0;nals=[]
    while at<len(mdat):
        length=int.from_bytes(mdat[at:at+4],'big');at+=4;nals.append(mdat[at:at+length]);at+=length
    assert len(nals)==6
    encoder,c=banks(1);ref=context(encoder,153);ic=context(encoder,154);ready={};order=[]
    encoder.decision(c['split0'],True)
    for x,y in [(0,0),(16,0),(0,16),(16,16)]:
        count=int((x-8,y)in ready)+int((x,y-8)in ready)
        encoder.decision(c[f'split{count}'],True)
        for dx,dy in [(0,0),(8,0),(0,8),(8,8)]:
            xx=x+dx;yy=y+dy;first=(xx,yy)==(0,0);order.append([xx,yy])
            count=int(ready.get((xx-8,yy),False))+int(ready.get((xx,yy-8),False))
            encoder.decision(c[f'skip{count}'],first)
            if first:merge_index_bins(encoder,c,2,5)
            else:
                encoder.decision(c['pred'],False);encoder.decision(c['part0'],True);encoder.decision(c['merge'],False)
                encoder.decision(ref,True);encoder.decision(c['mvd'],False);encoder.decision(c['mvd'],False);encoder.decision(c['mvp'],True)
            encoder.decision(ic,True)
            if not first:encoder.decision(c['root'],False)
            ready[(xx,yy)]=first
    header='1'+ue(2)+ue(1)+'1'+f'{1:08b}'+'0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0'+'1'
    header+='00'+'1'+ue(1)+'0'+'0'+'10'+ue(1)+se(0)+ue(1)+'00000000'+'1';header+='0'*((-len(header))%8)
    nals[5]=((1<<9)|(5<<3)|1).to_bytes(2,'big')+escape(packed(header)+encoder.finish())
    units=[nals[:3],nals[3:]];stream=b''.join(b'\0\0\0\1'+n for n in params+nals)
    pixels=[]
    for component in range(3):
        side=32 if component==0 else 16;cu=8 if component==0 else 4;out=[0]*(side*side);ready=set()
        def source(x,y):return(3*x+5*y+17)%256 if component==0 else(13*x+7*y+29+83*(component-1))%256
        for lx,ly in order:
            x=lx if component==0 else lx//2;y=ly if component==0 else ly//2;first=x==0 and y==0
            cur=[];ref_samples=[]
            if (x,y-cu)in ready:
                cur.extend(out[(y-1)*side+x+i]for i in range(cu));ref_samples.extend(source(x+i,y-1)for i in range(cu))
            if (x-cu,y)in ready:
                cur.extend(out[(y+i)*side+x-1]for i in range(cu));ref_samples.extend(source(x-1,y+i)for i in range(cu))
            weight,offset=parameters(cur,ref_samples,0)if component==0 and cur else(32,0)
            for yy in range(y,y+cu):
                for xx in range(x,x+cu):
                    value=source(xx,yy)
                    if first and component:value=(value+[71,113][component-1])%256
                    if component==0:value=min(255,max(0,min(255,(value*weight)>>5)+offset))
                    out[yy*side+xx]=value
            ready.add((x,y))
        pixels.extend(out)
    return {'illumination-small-cu.mp4':mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True),
        'illumination-small-cu.y16':struct.pack('<1536H',*pixels),
        'illumination-small-cu.json':(json.dumps(dict(provenance='Owned PCM/CABAC sixteen 8x8 IC CUs, independent luma boundary oracle and ordinary 4x4 chroma prediction. No private data or external codec.',cu_order=order),indent=2)+'\n').encode()}
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
