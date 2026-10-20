"""Owned coded depth IV acceptance/refusal movies, no external encoder."""
import json
from generate_hevc_depth_view_scaling import vps,sps,ue,se,ptl,header_pps,parameter_ids,layer,payload,bits,unescape,escape,packed,reference,inter_data,banks,terminal_zero
from generate_hevc_cqt import merge_index_bins,part,context
from generate_hevc_camera_parameters import camera_nal
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux

def header(id,poc,data,intra,maximum=5,iv=False):
    index={0:0,3:1,5:2,8:3}[id]
    value='1'+('0'if poc==0 else'')+ue(index)+ue(2 if intra else 1)+'1'
    if id or poc:value+=f'{poc:08b}'
    if poc:
        value+='0'+ue(0 if intra else 1)+ue(0)
        if not intra:value+=ue(0)+'1'
        value+=ue(0)+ue(0)+'0' # no LT entries, temporal MVP disabled
    if id in [5,8]:
        # Only depth view 0 is active for the target; source view 1 is unused.
        value+=('100'if id==8 and not intra else'0')
    value+='00'if id==0 else'0'
    if not intra:
        value+=('1'+ue(1)+'0'if id==8 else'0') # active count and optional list modification
        value+='0'+ue(5+int(iv)-maximum)
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(id<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def merged(size,index):
    b,c=banks(1);columns=size//32
    for row in range(columns):
        for col in range(columns):
            b.decision(c['split0'],False)
            b.decision(c[f'skip{int(col>0)+int(row>0)}'],True)
            merge_index_bins(b,c,index,5)
            if row*columns+col+1<columns**2:terminal_zero(b)
    return b.finish()

def split_source():
    b,c=banks(1);c['mvd1']=context(b,198)
    b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
    part(b,c,6,False,5) # left 8 columns / right 24 columns
    for index in range(2):
        b.decision(c['merge'],False)
        b.decision(c['mvd'],False);b.decision(c['mvd'],True)
        b.decision(c['mvd1'],False);b.bypass(index==1)
        b.decision(c['mvp'],index==1)
    b.decision(c['root'],False)
    return b.finish()

def movie(size,enabled,intra_source,index,direct,camera=True,camera_offset=0,source_split=False):
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    raw=bits(unescape(base[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000001')
    base=base[:2]+escape(packed(raw[:-10]+'0'+'1'))
    p0=header_pps(0,0,False);raw=bits(unescape(p0[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000000010')
    p0=p0[:2]+escape(packed(raw[:-13]+'0'+'1'))
    pps=[p0]+[parameter_ids(header_pps(id,0,False,constrained=direct),0,[i,i])for i,id in [(1,3),(2,5),(3,8)]]
    depth=[layer(sps(multilayer=True,sps_id=i,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),id)for i,id in [(1,3),(2,5)]]
    flags=[False]*14;flags[6]=enabled
    depth.append(layer(sps(multilayer=True,sps_id=3,three_d=payload(flags),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),8))
    video_vps=vps((100,102,108),size)
    if camera:video_vps=camera_nal(0,[(False,[]),(False,[(0,[0 if source_split else 1,camera_offset,0,0])])],original=video_vps)
    texture,_=reference(size,5,True,flat=True);pixels,_=reference(size,5,False)
    units=[[header(id,0,texture if id==0 else pixels,True)for id in [0,3,5,8]],
        [header(0,1,texture,True),header(3,1,pixels if intra_source else split_source()if source_split else inter_data(size,source=1),intra_source),
         header(5,1,pixels,True),header(8,1,merged(size,index),False,iv=enabled)]]
    stream=b''.join(b'\x00\x00\x00\x01'+n for n in [video_vps,base]+depth+pps+[n for au in units for n in au])
    return mux(stream,8,size,size,chroma_format=1,access_units=units,multilayer=True)

def generate():
    files={};cases=[]
    for size,direct in [(32,False),(128,False),(128,True)]:
        for enabled in [False,True]:
            for intra in [False,True]:
                for index in ([0,1,2,4]if size==32 else[0]):
                    name=f'depth-iv-{size}-'+('on'if enabled else'off')+('-intra'if intra else'')+('-direct'if direct else'')+f'-merge{index}.mp4'
                    files[name]=movie(size,enabled,intra,index,direct)
                    shift=(1 if (enabled and index in [0,2]or not enabled and index==1)and not intra else 0)
                    initial_index=index-1 if enabled and not intra else index
                    ref=1 if initial_index==1 else 0
                    if enabled and not intra and index==0:ref=0
                    cases.append(dict(file=name,size=size,direct=direct,enabled=enabled,intra_source=intra,index=index,shift=shift,motion=1 if enabled and not intra and index==0 else 0,reference=ref))
    for label,offset,shift in [('left',-24,1),('right',0,-1)]:
        name=f'depth-iv-camera-{label}.mp4'
        files[name]=movie(32,True,False,0,False,camera_offset=offset,source_split=True)
        cases.append(dict(file=name,size=32,direct=False,enabled=True,intra_source=False,index=0,source_split=True,shift=shift,motion=shift,reference=0))
    files['depth-iv-missing-camera.mp4']=movie(32,True,False,0,False,camera=False)
    files['depth-iv-manifest.json']=(json.dumps(dict(provenance='Owned PCM/CABAC four-layer MP4. Depth IV midpoint camera and same-AU source, source/target POC-0 temporal references. Exact on/off/index/intra/direct controls. No private media, FFmpeg, network or test-time generation.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
