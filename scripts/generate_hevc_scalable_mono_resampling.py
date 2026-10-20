"""Owned three-layer scalable monochrome reference resampling and temporal MVP.

The enhancement PTL signals Scalable Main 4:4:4 (8-bit constraints), carrying
monochrome layers; the ordinary base signals Main. This is not Annex I depth.
Independent PCM/CABAC authors and rational FIR oracle; no private media, codec,
FFmpeg or network. Generation is separate from ordinary tests.
"""
import struct
from generate_hevc_layered_sps import ue,ptl,representation,sps
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_layered_reconstruction import parameter_ids,layer
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_cqt import reference,predicted,banks,context,terminal_zero,nal
from generate_hevc_wedge_tables import OUT
from generate_hevc_inter_layer_resampling import derive,oracle,L
from hevc_fixture_mp4 import mux

def vps():
    formats=[dict(size=size,chroma=chroma,separate=False,depth=[8,8],crop=[0]*4)
        for size,chroma in [([64,64],1),([32,32],0),([64,64],0)]]
    b='0000'+'11'+f'{2:06b}'+'0101'+'1'*16+ptl(True)
    b+='0'+ue(15)+ue(2)+ue(0)+f'{5:06b}'+ue(1)+'100101'+'0'+'1'
    b+='1'*((-len(b))%8)+ptl(False,level=120)
    b+='0'+''.join('1'if i==2 else'0'for i in range(16))+'001'+'1'
    b+=f'{3:06b}'+'01'+f'{5:06b}'+'10'+'0000'+'101'
    constraints=sum(1<<bit for bit in [43,42,41,35,34]) # Table H.4, Scalable Main 4:4:4
    extra=f'{10:08b}'+'0'*32+f'{constraints:048b}'+f'{120:08b}'+'0'*16
    b+='000'+ue(2)+'1'+extra+ue(0)+'00'+'01'+'10'+'10'
    b+=ue(2)+''.join(representation(f)for f in formats)+'1'+'01'+'10'+'01'
    b+='1'
    for tid in range(3):
        if tid:b+='1'
        b+=''.join(ue(15)for _ in range(3))+ue(2+tid)+ue(0)
    b+=ue(0)+'1'+'10'+ue(0)+'0'+'0'+'1'
    return b'\x40\x01'+escape(packed(b))

def target_header(poc,data,temporal=False):
    b='1'+('0'if poc==0 else'')+ue(2)+ue(2 if poc==0 else 1)+'1'+f'{poc:08b}'
    if poc:b+='0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+str(int(temporal))
    b+=str(int(poc>0)) # enable sole same-role inter-layer reference
    # No direct texture dependency for layer 5: in-component flag is absent.
    b+='0' # luma SAO off
    if poc:
        b+='1'+ue(1) # two active L0 entries: own previous, current source layer
        b+='0' # list modification off with two total current references
        b+='0' # CABAC init swap off
        if temporal:b+=ue(1) # collocated active index 1 = prepared layer 3
        b+=ue(4) # one merge candidate
    b+=se(0)+ue(1)+'00000000'+'1';b+='0'*((-len(b))%8)
    return (((19 if poc==0 else 1)<<9)|(5<<3)|1).to_bytes(2,'big')+escape(packed(b)+data)

def target_inter(poc,reference_index):
    b,c=banks(1);ref=context(b,153)
    for index in range(4):
        b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
        b.decision(c['part0'],True);b.decision(c['merge'],False)
        b.decision(ref,reference_index==1)
        b.decision(c['mvd'],False);b.decision(c['mvd'],False);b.decision(c['mvp'],False)
        b.decision(c['root'],False)
        if index<3:terminal_zero(b)
    return b.finish()

def source_inter(poc,displacement=4):
    # One 32x32 CU, coded quarter-sample MVD. POC2 uses zero MVD.
    b,c=banks(1)
    b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
    b.decision(c['part0'],True);b.decision(c['merge'],False)
    value=displacement if poc==1 else 0
    b.decision(c['mvd'],False);b.decision(c['mvd'],value!=0)
    if value:
        gt1=context(b,198);b.decision(gt1,abs(value)>1)
        if abs(value)>1:
            remaining=abs(value)-2;k=1
            while remaining>=(1<<k):b.bypass(True);remaining-=1<<k;k+=1
            b.bypass(False)
            for shift in reversed(range(k)):b.bypass((remaining>>shift)&1)
        b.bypass(value<0)
    b.decision(c['mvp'],False);b.decision(c['root'],False)
    return b.finish()

def generate_case(integer_scale=False,displacement=4):
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(64,64),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    source=sps(multilayer=True,sps_id=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    target=layer(sps(multilayer=True,sps_id=2,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),5)
    from generate_hevc_colour_plane_fixtures import bits,unescape
    body=bits(unescape(base[2:]));body=body[:body.rfind('1')]
    assert body.endswith('1010000001') # extension-present, multilayer flag, constraint
    base=base[:2]+escape(packed(body[:-10]+'0'+'1')) # ordinary Main base SPS
    base_pps=header_pps(0,0,False)
    body=bits(unescape(base_pps[2:]));body=body[:body.rfind('1')]
    assert body.endswith('1010000000010')
    base_pps=base_pps[:2]+escape(packed(body[:-13]+'0'+'1'))
    pps=[base_pps,parameter_ids(header_pps(3,0,False),0,[1,1])]
    # Preserve common PPS fields and author one layer-3 scaled reference region.
    raw=header_pps(5,0,False);body=bits(unescape(raw[2:]));body=body[:body.rfind('1')]
    assert body.endswith('00'+ue(0)+'0')
    location='00'+ue(1)+f'{3:06b}'+'1'+''.join(se(0 if integer_scale else 8)for _ in range(4))+'0'+'0'+'0'
    target_pps=raw[:2]+escape(packed(body[:-4]+location+'1'))
    target_pps=parameter_ids(target_pps,0,[2,2]);pps.append(target_pps)
    tex,_=reference(64,5,True,flat=True);src,source_pixels=reference(32,5,False);dst,target_pixels=reference(64,5,False)
    groups=[]
    for poc in range(3):
        texdata=tex if poc==0 else predicted(64,5,False,poc,0,True,False,'skip')
        # P-style POC2 source, retaining a zero predictor and previous source pixels.
        srcnal=nal(3,poc,src if poc==0 else source_inter(poc,displacement),False,stype=2 if poc==0 else 1)
        # nal() uses B header default at POC2; explicit stype=1 keeps its syntax P.
        groups.append([nal(0,poc,texdata,False),srcnal,
            target_header(poc,dst if poc==0 else target_inter(poc,0 if poc==1 else 1),temporal=poc==1)])
    params=[vps(),base,source,target]+pps
    stream=b''.join(b'\x00\x00\x00\x01'+n for n in params+[n for au in groups for n in au])
    projected=displacement if displacement!=4 else 8 if integer_scale else 6
    taps=L[(projected%4)*4];origin=projected//4
    first=[min(255,max(0,(sum(target_pixels[min(63,max(0,y+origin+k-3))*64+x]*weight
        for k,weight in enumerate(taps))+32)//64))for y in range(64)for x in range(64)]
    source_shift=1 if displacement==4 else displacement
    source_shifted=[source_pixels[min(31,max(0,y+source_shift))*32+x]for y in range(32)for x in range(32)]
    config=dict(reference_format=0,current_format=0,reference_size=[32,32],current_size=[64,64],
        reference_depth=[8,8],current_depth=[8,8],scaled=[0 if integer_scale else 8]*4,region=[0]*4,phases=None)
    final=oracle(config,[source_shifted,[],[]],derive(config))[0]
    stem='scalable-mono-resampling'+('-integer-scale'if integer_scale else'')+('-positive-limit'if displacement==32767 else'-negative-limit'if displacement==-32768 else'')
    return {stem+'.mp4':mux(stream,8,64,64,chroma_format=1,access_units=groups,multilayer=True),
        stem+'.y16':struct.pack('<12288H',*(target_pixels+first+final))}

def generate():
    files={}
    for integer_scale in [False,True]:
        for displacement in [4,32767,-32768]:files.update(generate_case(integer_scale,displacement))
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
