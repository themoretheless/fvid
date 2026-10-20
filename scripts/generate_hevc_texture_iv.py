"""Owned texture IV/DI native-dispatch witnesses; no external encoder."""
import json,struct
import generate_hevc_inter_view_scaling as p
from generate_hevc_cqt import texture_amp,banks,merge_index_bins,reference,ue,se,OUT
from generate_hevc_colour_plane_fixtures import bits,unescape,escape,packed
from generate_hevc_layered_sps import sps
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_reconstruction import parameter_ids,layer
from generate_hevc_3d_sps import payload
from hevc_fixture_mp4 import mux

def target_header(data,poc,enabled=False):
    if poc==0:return p.header(5,0,data)
    value='1'+ue(2)+ue(1)+'1'+f'{poc:08b}'
    value+='0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0'
    value+='101' # enable IL, one reference, select layer3 only
    value+='00' # SAO
    value+='1'+ue(1)+'0' # active L0=2, default list [own past, layer3 current]
    value+='0'+'0'+ue(int(enabled)) # CABAC init, IC off, five candidates from 5+NumExtra
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return ((1<<9)|(5<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def target(index):
    encoder,contexts=banks(1)
    encoder.decision(contexts['split0'],False);encoder.decision(contexts['skip0'],True)
    merge_index_bins(encoder,contexts,index,5)
    return encoder.finish()

def movie(enabled,intra,index,top=8):
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(32,32),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    raw=bits(unescape(base[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000001')
    base=base[:2]+escape(packed(raw[:-10]+'0'+'1'))
    source=layer(sps(multilayer=True,sps_id=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),3)
    flags=[False]*14;flags[0]=enabled
    destination=layer(sps(multilayer=True,sps_id=2,three_d=payload(flags),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),5)
    first_pps=header_pps(0,0,False);raw=bits(unescape(first_pps[2:]));raw=raw[:raw.rfind('1')];assert raw.endswith('1010000000010')
    first_pps=first_pps[:2]+escape(packed(raw[:-13]+'0'+'1'))
    params=[p.vps((100,102,108)),base,source,destination,first_pps,
        parameter_ids(header_pps(3,0,False),0,[1,1]),parameter_ids(header_pps(5,0,False),0,[2,2])]
    pcm,_=reference(32,5,True)
    units=[[p.header(id,0,pcm)for id in [0,3,5]],
        [p.header(0,1,pcm),p.header(3,1,pcm if intra else texture_amp(32,4,top=top),all_intra=intra),target_header(target(index),1,enabled)]]
    stream=b''.join(b'\0\0\0\1'+nal for nal in params+[n for au in units for n in au])
    return mux(stream,8,32,32,chroma_format=1,access_units=units,multilayer=True)

def oracle(enabled,intra,index,top=8):
    # Initial list: five zero candidates, refs 0/1/0/0/0.
    # Inter source IV default is -1 vertical, while its top subblocks use +1.
    # IVShift is available but equal to IV default, suppressing DIShift.
    labels=(['IV','DI','past','source','past']if not intra else['DI','DIShift','past','source','past'])if enabled else['past','source','past','past','past']
    if enabled and not intra and top==24:labels=['IV','DI','IVShift','past','source']
    label=labels[index]
    values=[]
    for y in range(32):
        for x in range(32):
            def pattern(xx,yy):return(3*xx+5*yy+17)%256
            def source(xx,yy):
                xx=min(31,max(0,xx));yy=min(31,max(0,yy))
                shift=0 if intra else(1 if yy<top else -1)
                return pattern(xx,min(31,max(0,yy+shift)))
            if label=='past':value=pattern(x,y)
            elif label=='IV':value=source(x,min(31,max(0,y+(1 if y<top else -1))))
            elif label=='IVShift':value=source(x,y-1)
            elif label=='DIShift':value=source(x+1,y)
            else:value=source(x,y)
            values.append(value)
    return label,values

def generate():
    files={};cases=[];golden=bytearray()
    for enabled in [False,True]:
        for intra in [False,True]:
            for top in ([8]if intra else[8,24]):
                for index in range(5):
                    name='texture-iv-'+('on'if enabled else'off')+('-intra'if intra else'')+('-bottom8'if top==24 else'')+f'-{index}.mp4'
                    files[name]=movie(enabled,intra,index,top)
                    label,pixels=oracle(enabled,intra,index,top);offset=len(golden)//2
                    golden.extend(struct.pack('<1024H',*pixels))
                    cases.append(dict(file=name,enabled=enabled,intra=intra,top=top,index=index,label=label,offset=offset))
    files['texture-iv-oracles.y16']=bytes(golden)
    files['texture-iv-manifest.json']=(json.dumps(dict(
        provenance='Owned PCM/CABAC, parameters, MP4 and scalar pixel oracle. No private media, FFmpeg, network or test-time generation.',
        layers=[0,3,5],views=[0,1,2],source_amp_rows=[[8,24],[24,8]],source_mv_quarters=[4,-4],
        target_lists=['own POC0','layer3 POC1'],default_reference_view=1,disparity=[0,0],
        limitations='Native 8-bit 420 texture IV/DI P-slice acceptance and disabled/intra controls. Neutral chroma, 32x32 single-CU witnesses; not full Annex I conformance.',
        cases=cases),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
