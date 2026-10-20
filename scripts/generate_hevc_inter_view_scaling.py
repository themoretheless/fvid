"""Owned three-view temporal MV scaling witness; no external codec or private data."""
import json
from generate_hevc_layered_sps import ue,ptl,representation,sps
from generate_hevc_layered_pps import se
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_reconstruction import parameter_ids,layer
from generate_hevc_colour_plane_fixtures import packed,escape,bits,unescape
from generate_hevc_3d_sps import payload
from generate_hevc_cqt import reference,banks,context
from generate_hevc_scalable_mono_resampling import source_inter
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux

def vps(views):
    fmt=dict(size=[32,32],chroma=1,separate=False,depth=[8,8],crop=[0]*4)
    b='0000'+'11'+f'{2:06b}'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(15)+ue(2)+ue(0)+f'{5:06b}'+ue(1)+'100101'+'0'+'1'
    b+='1'*((-len(b))%8)+ptl(False,level=120)
    b+='0'+''.join('1'if i==1 else'0'for i in range(16))+'001'+'1'
    width=max(views).bit_length()
    assert 0<width<=15
    b+=f'{3:06b}'+'01'+f'{5:06b}'+'10'+f'{width:04b}'+''.join(f'{v:0{width}b}'for v in views)+'111'
    constraints=sum(1<<bit for bit in [43,42,41,40,39,35])
    extra=f'{8:08b}'+'0'*32+f'{constraints:048b}'+f'{120:08b}'+'0'*16
    b+='000'+ue(2)+'1'+extra+ue(0)+'00'+'01'+'10'+'10'
    b+=ue(2)+representation(fmt)*3+'1'+'01'+'10'+'01'+'1'
    for tid in range(3):
        if tid:b+='1'
        b+=''.join(ue(15)for _ in range(3))+ue(2+tid)+ue(0)
    b+=ue(0)+'1'+'10'+ue(0)+'0' # no VPS VUI
    b+='11' # vps_extension2_flag, vps_3d_extension_flag (I.7.4.3.1)
    b+='1'*((-len(b))%8)
    b+=ue(0)+'000000'*2+'0'+'1' # cp_precision=0, no CP pairs, extension3=0
    return b'\x40\x01'+escape(packed(b))

def header(id,poc,data,all_intra=False,camera=None):
    intra=poc==0 or id==0 or all_intra;temporal=id==5 and poc==1 and not intra
    b='1'+('0'if poc==0 else'')+ue({0:0,3:1,5:2}[id])+ue(2 if intra else 1)+'1'
    if id or poc:b+=f'{poc:08b}'
    if poc:b+='0'+ue(0)+ue(0)+ue(0)+ue(0)+str(int(temporal))
    if id:
        b+=str(int(poc>0 and not intra))
        if id==5 and poc and not intra:b+='1' # both direct inter-layer references active
    b+='00' # 4:2:0 SAO off, no depth component/in-component syntax
    if not intra:
        b+=('1'+ue(1)+'0')if id==5 else'0' # two active L0, no list modification
        b+='0' # CABAC init
        if temporal:b+=ue(1) # collocated layer 3, target ref 0 = layer 0
        b+='0' # slice_ic_enabled_flag, Annex I texture reference layers
        b+=ue(4)
    b+=se(0)
    if camera is not None:
        for values in camera.get(id,[]):b+=''.join(se(v)for v in values)
    b+=ue(1)+'00000000'+'1';b+='0'*((-len(b))%8)
    return (((19 if poc==0 else 1)<<9)|(id<<3)|1).to_bytes(2,'big')+escape(packed(b)+data)

def target_inter(merge=False):
    b,c=banks(1);ref=context(b,153)
    b.decision(c['split0'],False);b.decision(c['skip0'],merge)
    if merge:return b.finish()
    b.decision(c['pred'],False)
    b.decision(c['part0'],True);b.decision(c['merge'],False)
    b.decision(ref,False)
    b.decision(c['mvd'],False);b.decision(c['mvd'],False);b.decision(c['mvp'],False)
    b.decision(c['root'],False)
    return b.finish()

CASES=[
    ('',(100,102,108),4,16),
    ('negative',(100,102,96),4,-8),
    ('equal',(7,7,7),4,4),
    ('zero-source',(100,100,108),4,4),
    ('zero-target',(100,102,100),4,4),
    ('positive-limit',(100,102,108),32767,32767),
    ('negative-limit',(100,102,108),-32768,-32768),
    ('factor-clip',(0,1,32767),4,64),
]
def generate_case(label,views,displacement,merge=False,all_intra=False,camera=None,chroma_pattern=False):
    files={}
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(32,32),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    raw=bits(unescape(base[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000001')
    base=base[:2]+escape(packed(raw[:-10]+'0'+'1'))
    source=sps(multilayer=True,sps_id=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    p0=header_pps(0,0,False);raw=bits(unescape(p0[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000000010')
    p0=p0[:2]+escape(packed(raw[:-13]+'0'+'1'))
    pps=[p0,parameter_ids(header_pps(3,0,False),0,[1,1]),parameter_ids(header_pps(5,0,False),0,[2,2])]
    pcm,_=reference(32,5,True,chroma_pattern=chroma_pattern)
    for enabled in [False,True]:
        flags=[False]*14;flags[1]=enabled
        target=layer(sps(multilayer=True,sps_id=2,three_d=payload(flags),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),5)
        pictures=[[header(id,0,pcm,all_intra,camera)for id in [0,3,5]],
            [header(0,1,pcm,all_intra,camera),header(3,1,pcm if all_intra else source_inter(1,displacement),all_intra,camera),header(5,1,pcm if all_intra else target_inter(merge),all_intra,camera)]]
        stream=b''.join(b'\x00\x00\x00\x01'+n for n in [vps(views),base,source,target]+pps+[n for au in pictures for n in au])
        files['inter-view-temporal-scaling-'+('on'if enabled else'off')+('-'+label if label else'')+('-merge'if merge else'')+'.mp4']=mux(stream,8,32,32,chroma_format=1,access_units=pictures,multilayer=True)
    return files

def generate():
    files={}
    for label,views,source,_ in CASES:
        for merge in [False,True]:files.update(generate_case(label,views,source,merge))
    manifest=dict(provenance='Owned PCM, CABAC, VPS/SPS/PPS and MP4 authors. No external codec, private material or network. Tests do not generate fixtures.',
        layers=[0,3,5],view_indices=[0,1,2],profile=[1,8,8],size=[32,32],chroma=1,depth=8,
        vps_3d_extension=True,cp_precision=0,num_cp=[0,0],
        modes=['explicit AMVP','skip temporal merge'],
        scaling_applies_to='skip temporal merge only; explicit AMVP copies long-term motion',
        cases=[dict(label=label,view_ids=views,source_mv=[0,source],scaled_mv=[0,target])for label,views,source,target in CASES])
    files['inter-view-temporal-scaling-manifest.json']=(json.dumps(manifest,indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
