"""Owned four-layer / three-depth-view temporal scaling; offline PCM/CABAC only."""
import json
from generate_hevc_layered_sps import ue,ptl,representation,sps
from generate_hevc_layered_pps import se
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_reconstruction import parameter_ids,layer
from generate_hevc_colour_plane_fixtures import packed,escape,bits,unescape
from generate_hevc_3d_sps import payload
from generate_hevc_cqt import reference,banks,context,terminal_zero
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux

CASES=[
    ('',(100,102,108),1,4),
    ('negative',(100,102,96),1,-2),
    ('equal',(7,7,7),1,1),
    ('zero-source',(100,100,108),1,1),
    ('zero-target',(100,102,100),1,1),
    ('positive-limit',(100,102,108),32767,32767),
    ('negative-limit',(100,102,108),-32768,-32768),
    ('factor-clip',(0,1,32767),1,16),
    ('integer-round',(100,103,102),1,1),
    ('negative-round',(100,103,102),-1,-1),
]
def vps(views,size=32):
    texture=dict(size=[size,size],chroma=1,separate=False,depth=[8,8],crop=[0]*4)
    depth=dict(texture,chroma=0)
    b='0000'+'11'+f'{3:06b}'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(15)+ue(2)+ue(0)+f'{8:06b}'+ue(1)+'100101001'+'0'+'1'
    b+='1'*((-len(b))%8)+ptl(False,level=120)
    b+='0'+'11'+'0'*14+'000'+'001'+'1'
    for id,view in [(3,0),(5,1),(8,2)]:b+=f'{id:06b}'+'1'+f'{view:02b}'
    width=max(views).bit_length();assert 0<width<=15
    b+=f'{width:04b}'+''.join(f'{v:0{width}b}'for v in views)+'101011'
    constraints=sum(1<<bit for bit in [43,42,41,40,39,35])
    extra=f'{8:08b}'+'0'*32+f'{constraints:048b}'+f'{120:08b}'+'0'*16
    b+='000'+ue(2)+'1'+extra+ue(0)+'00'+'01'+'10'*3
    b+=ue(1)+representation(texture)+representation(depth)+'1'+'111'+'01'+'1'
    for tid in range(3):
        if tid:b+='1'
        b+=''.join(ue(15)for _ in range(4))+ue(2+tid)+ue(0)
    b+=ue(0)+'1'+'10'+ue(0)+'0'+'11'
    b+='1'*((-len(b))%8)
    b+=ue(0)+'000000'*2+'0'+'1'
    return b'\x40\x01'+escape(packed(b))

def header(id,poc,data):
    index={0:0,3:1,5:2,8:3}[id]
    intra=poc==0 or id in [0,3];temporal=id==8 and poc==1
    b='1'+('0'if poc==0 else'')+ue(index)+ue(2 if intra else 1)+'1'
    if id or poc:b+=f'{poc:08b}'
    if poc:b+='0'+ue(0)+ue(0)+ue(0)+ue(0)+str(int(temporal))
    if id in [5,8]:
        b+=str(int(poc>0))
        if id==8 and poc:b+='1'
    b+='00'if id==0 else'0'
    if not intra:
        b+=('1'+ue(1)+'0')if id==8 else'0'
        b+='0'
        if temporal:b+=ue(1)
        b+=ue(4)
    b+=se(0)+ue(1)+'00000000'+'1';b+='0'*((-len(b))%8)
    return (((19 if poc==0 else 1)<<9)|(id<<3)|1).to_bytes(2,'big')+escape(packed(b)+data)

def inter_data(size,source=None,merge=False):
    b,c=banks(1);ref=context(b,153);gt1=context(b,198)
    cols=size//32
    for cy in range(cols):
        for cx in range(cols):
            b.decision(c['split0'],False)
            b.decision(c[f'skip{int(cx>0)+int(cy>0)if merge else 0}'],merge)
            if not merge:
                b.decision(c['pred'],False);b.decision(c['part0'],True);b.decision(c['merge'],False)
                if source is None:b.decision(ref,False)
                value=source if source is not None and cx==0 and cy==0 else 0
                b.decision(c['mvd'],False);b.decision(c['mvd'],value!=0)
                if value:
                    b.decision(gt1,abs(value)>1)
                    if abs(value)>1:
                        remaining=abs(value)-2;k=1
                        while remaining>=1<<k:b.bypass(True);remaining-=1<<k;k+=1
                        b.bypass(False)
                        for shift in reversed(range(k)):b.bypass((remaining>>shift)&1)
                    b.bypass(value<0)
                b.decision(c['mvp'],False);b.decision(c['root'],False)
            if cy*cols+cx+1<cols*cols:terminal_zero(b)
    return b.finish()

def generate_geometry(size=32,direct=False):
    files={}
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    raw=bits(unescape(base[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000001')
    base=base[:2]+escape(packed(raw[:-10]+'0'+'1'))
    p0=header_pps(0,0,False);raw=bits(unescape(p0[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000000010')
    p0=p0[:2]+escape(packed(raw[:-13]+'0'+'1'))
    pps=[p0]+[parameter_ids(header_pps(id,0,False,constrained=direct),0,[index,index])for index,id in [(1,3),(2,5),(3,8)]]
    depth_params=[layer(sps(multilayer=True,sps_id=index,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),id)for index,id in [(1,3),(2,5)]]
    tex,_=reference(size,5,True,flat=True);dep,_=reference(size,5,False)
    for label,views,source,target in CASES:
        for enabled in [False,True]:
            flags=[False]*14;flags[7]=enabled
            last=layer(sps(multilayer=True,sps_id=3,three_d=payload(flags),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),8)
            for merge in [False,True]:
                pictures=[[header(id,0,tex if id==0 else dep)for id in [0,3,5,8]],
                    [header(0,1,tex),header(3,1,dep),header(5,1,inter_data(size,source=source)),header(8,1,inter_data(size,merge=merge))]]
                stream=b''.join(b'\x00\x00\x00\x01'+n for n in [vps(views,size),base]+depth_params+[last]+pps+[n for au in pictures for n in au])
                name='depth-view-temporal-scaling'+(''if size==32 else f'-{size}')+'-'+('on'if enabled else'off')+('-'+label if label else'')+('-merge'if merge else'')+('-direct'if direct else'')+'.mp4'
                files[name]=mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)
    return files

def generate():
    files={}
    for size,direct in [(32,False),(128,False),(128,True)]:files.update(generate_geometry(size,direct))
    manifest=dict(provenance='Owned PCM/CABAC, parameters and MP4 only; no private source, external codec, network or test-time generation.',
        scaling_applies_to='skip temporal merge only; explicit AMVP copies long-term motion',
        layers=[0,3,5,8],view_indices=[0,0,1,2],depth_roles=[0,1,1,1],profile=[1,8,8,8],
        geometries=[dict(size=32,direct=False),dict(size=128,direct=False),dict(size=128,direct=True)],vps_3d_extension=True,cp_precision=0,num_cp=[0,0],
        motion_domain='signed 16-bit integer depth samples; shared storage is multiplied by four',
        cases=[dict(label=label,views=views,source=source,target=target)for label,views,source,target in CASES])
    files['depth-view-temporal-scaling-manifest.json']=(json.dumps(manifest,indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
