"""Owned one-AU texture/depth MP4 playback acceptance fixtures, fully offline."""
from generate_he_aac_packet_fixtures import boxes,DEST
from generate_hevc_layered_reconstruction import configuration,parameter_ids
from generate_hevc_colour_plane_fixtures import bits,unescape,escape,packed
from generate_hevc_layered_sps import vps
from generate_hevc_layered_pps import se,ue
from generate_hevc_mixed_skip import generate as mixed
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux
def generate(wedge=False,dlt=False,ordinary=None,component=None,contour=False,residual=None,dim_mode="contour",dim_offset=10,invalid_dim_split=False,nxn_schedule=None,nxn_residual=False,nxn_size=32,nxn_invalid_split=False,nxn_ordinary_mode=1,nxn_ordinary_split=False):
    source=(DEST/'hevc-pcm-reference-rext8.mp4').read_bytes()
    def child(d,t):return next(p for k,p in boxes(d)if k==t)
    moov=child(source,b'moov');stbl=child(child(child(child(moov,b'trak'),b'mdia'),b'minf'),b'stbl')
    entry=next(boxes(child(stbl,b'stsd')[8:]))[1]
    params=configuration(child(entry[78:],b'hvcC'))
    stsz=child(stbl,b'stsz');length=int.from_bytes(stsz[12:16],'big')
    offset=int.from_bytes(child(stbl,b'stco')[8:12],'big');packet=source[offset:offset+length]
    base=[];at=0
    while at<len(packet):
        n=int.from_bytes(packet[at:at+4],'big');at+=4;base.append(packet[at:at+n]);at+=n
    assert at==len(packet)
    split=ordinary is not None and ordinary.startswith("split-")
    if wedge:
        from generate_hevc_wedge_picture import generate as wedge_picture
        owned=wedge_picture()
        if split:
            from generate_hevc_dc_split import generate as dc_split
            owned.update(dc_split())
    else:owned=mixed()
    if contour:
        from generate_hevc_contour_picture import generate as contour_picture
        owned.update(contour_picture(residual,dim_mode,dim_offset,invalid_dim_split))
    if nxn_schedule:
        from generate_hevc_nxn_dim import generate as nxn_dim
        owned.update(nxn_dim(nxn_schedule,nxn_residual,nxn_size,nxn_invalid_split,nxn_ordinary_mode,nxn_ordinary_split))
    if component:
        from generate_hevc_component_gate import generate as component_gate
        owned.update(component_gate())
    # The selected base representation is the source's authored 64x64 8-bit 420.
    target=dict(chroma=0,separate=False,depth=[8,8]if wedge else[10,10],size=[64,64]if component or split else[32,32]if wedge else[64,64],crop=[0]*4)
    if nxn_schedule:target["size"]=[nxn_size,nxn_size]
    base_format=dict(chroma=1,separate=False,depth=[8,8],size=[64,64],crop=[0]*4)
    raw_vps=vps(target,depth_layer=True,base_format=base_format)[0]
    v=bits(unescape(raw_vps[2:]));ptl=bits(unescape(params[33][0][2:]))[8:104]
    raw_vps=raw_vps[:2]+escape(packed(v[:32]+ptl+v[128:]))
    depth_sps=parameter_ids(owned['contour-picture.sps'if contour else 'component-gate.sps'if component else f'dc-{ordinary}.sps'if split else 'wedge-picture.sps'if wedge else'mixed-skip.sps'],7,[1])
    depth_pps=parameter_ids(owned['wedge-dlt.pps'if dlt else 'wedge-picture.pps'if wedge else'mixed-skip.pps'],0,[1,1])
    if contour:
        from generate_hevc_layered_slices import header_pps
        from generate_hevc_depth_tables import header,delta
        tables=header(8,1)+'100'+delta(8,list(range(0,256,8))) if dlt else None
        depth_pps=parameter_ids(header_pps(3,0,False,True,depth_tables=tables),0,[1,1])
        from generate_hevc_poc_reset_fixtures import reset_pps
        params[34][0]=reset_pps(params[34][0])
        # Native parser metadata for this authored seed: IDR header is three
        # bytes, no extension; retain every entropy/PCM byte unchanged.
        assert len(base)==1
        rbsp=unescape(base[0][2:]);assert rbsp[:3]==bytes.fromhex('af06b8')
        header=bits(rbsp[:3]);header=header[:header.rfind('1')]
        header+=ue(2)+'1000000000000000'+'1'
        header+='0'*((-len(header))%8)
        base=[base[0][:2]+escape(packed(header)+rbsp[3:])]
    files={}
    for mode in range(1 if wedge else 4):
        original=(((21<<9)|(3<<3)|1).to_bytes(2,'big')+escape(owned['contour-picture.cabac']))if contour else owned[f'component-gate-{component}.nal'if component else 'dc-split.nal'if split else f'ordinary-dc-{ordinary}.nal'if ordinary else 'wedge-picture.nal'if wedge else f'mixed-skip-mode{mode}-high.nal'];raw=unescape(original[2:])
        prefix='10'+ue(1)+ue(2)+'1'+'00000000'+(str(int(component=='on'))if component else'')+'0'+se(0)+ue(0)+'1'
        if contour:
            prefix='10'+ue(1)+ue(2)+'1'+'00000000'+'0'+ue(0)+ue(0)+ue(0)+ue(0)+'0'+'1'+'0'+se(0)+ue(2)+'1000000000000000'+'1'
        prefix+='0'*((-len(prefix))%8)
        assert contour or len(prefix)==24
        depth=original[:2]+escape(packed(prefix)+(raw if contour else raw[3:]))
        parameters=[raw_vps,params[33][0],depth_sps,params[34][0],depth_pps]
        pictures=base+[depth]
        stream=b''.join(b'\x00\x00\x00\x01'+n for n in parameters+pictures)
        name=(f'{dim_mode}-residual-'+{0:'zero',64:'positive',-64:'negative'}[residual]+('-clip'if dim_offset==150 else'')+('-dlt'if dlt else'')+'-playback.mp4')if residual is not None else None
        if nxn_schedule:name=('nxn4-'if nxn_size==8 else'nxn-')+f'{nxn_schedule}-'+('mixed'if nxn_residual else'zero')+('-dlt'if dlt else'')+'-playback.mp4'
        if nxn_schedule and (nxn_ordinary_mode!=1 or nxn_ordinary_split):
            name=f'nxn-mode{nxn_ordinary_mode}-'+('split'if nxn_ordinary_split else'whole')+f'-{nxn_schedule}-'+('mixed'if nxn_residual else'zero')+('-dlt'if dlt else'')+'-playback.mp4'
        if nxn_invalid_split:name=f'nxn-{nxn_schedule}-split-playback.mp4'
        if invalid_dim_split:name=f'{dim_mode}-residual-split-playback.mp4'
        files[name if name else('contour-dlt-playback.mp4'if dlt else'contour-playback.mp4')if contour else(f'in-component-{component}-playback.mp4')if component else(f'ordinary-dc-{ordinary}'+('-dlt'if dlt else'')+'-playback.mp4')if ordinary else 'wedge-dlt-playback.mp4'if dlt else 'wedge-playback.mp4'if wedge else f'skip-playback-mode{mode}.mp4']=mux(stream,8,64,64,chroma_format=1,access_units=[pictures],multilayer=True)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for kind,dlt,ordinary in [(False,False,None),(True,False,None),(True,True,None)]+[(True,dlt,ordinary)for dlt in [False,True]for ordinary in ["present","absent","split-tb2","split-tb3","split-tb4","split-tb5"]]:
        for n,b in generate(kind,dlt,ordinary).items():(OUT/n).write_bytes(b)
    for component in ["off","on"]:
        for n,b in generate(True,False,None,component).items():(OUT/n).write_bytes(b)

    for dlt in [False,True]:
        for n,b in generate(True,dlt,contour=True).items():(OUT/n).write_bytes(b)

    for mode in ["contour","wedge"]:
        for residual in [0,64,-64]:
            for dlt in [False,True]:
                for n,b in generate(True,dlt,contour=True,residual=residual,dim_mode=mode).items():(OUT/n).write_bytes(b)

        for residual in [64,-64]:
            for dlt in [False,True]:
                for n,b in generate(True,dlt,contour=True,residual=residual,dim_mode=mode,dim_offset=150).items():(OUT/n).write_bytes(b)
        for n,b in generate(True,contour=True,residual=0,dim_mode=mode,invalid_dim_split=True).items():(OUT/n).write_bytes(b)

    from generate_hevc_nxn_dim import SCHEDULES
    for schedule in SCHEDULES:
        for residual in [False,True]:
            for dlt in [False,True]:
                for n,b in generate(True,dlt,contour=True,nxn_schedule=schedule,nxn_residual=residual).items():(OUT/n).write_bytes(b)

    for schedule in SCHEDULES:
        for residual in [False,True]:
            for dlt in [False,True]:
                for n,b in generate(True,dlt,contour=True,nxn_schedule=schedule,nxn_residual=residual,nxn_size=8).items():(OUT/n).write_bytes(b)

    for schedule in ['owco','wcwc']:
        for n,b in generate(True,contour=True,nxn_schedule=schedule,nxn_invalid_split=True).items():(OUT/n).write_bytes(b)
