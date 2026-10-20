"""Owned mixed texture/depth topology for texture disparity refinement, offline.

The control qualifies component binding; on/off DI pictures qualify native refinement.
"""
from generate_hevc_layered_sps import ue, ptl, representation
from generate_hevc_colour_plane_fixtures import packed, escape
from generate_hevc_camera_parameters import camera_bits
from generate_hevc_wedge_tables import OUT


def vps(scale=128, camera_available=True, offset=0, precision=0, size=32):
    texture = dict(size=[size, size], chroma=1, separate=False, depth=[8, 8], crop=[0]*4)
    depth = dict(texture, chroma=0)
    b = '0000'+'11'+f'{2:06b}'+'0101'+'1'*16+ptl(True)
    b += '0'+ue(15)+ue(2)+ue(0)+f'{5:06b}'+ue(1)+'100101'+'0'+'1'
    b += '1'*((-len(b))%8)+ptl(False, level=120)
    # Independent depth and view scalability dimensions (1 and 2 bits).
    b += '0'+'11'+'0'*14+'000'+'001'+'1'
    # Layer 0 = texture view 0, 3 = depth view 0, 5 = texture view 1.
    b += f'{3:06b}'+'1'+'00'+f'{5:06b}'+'0'+'01'
    b += '0001'+'0'+'1'+'111'  # view IDs, all three direct dependency edges
    constraints = sum(1 << bit for bit in [43,42,41,40,39,35])
    extra = f'{8:08b}'+'0'*32+f'{constraints:048b}'+f'{120:08b}'+'0'*16
    b += '000'+ue(2)+'1'+extra+ue(0)+'00'+'01'+'10'*2
    # Two representation formats; explicit layer indices 1 (depth), 0 (texture).
    b += ue(1)+representation(texture)+representation(depth)+'1'+'10'+'01'+'1'
    for tid in range(3):
        if tid: b += '1'
        b += ''.join(ue(15) for _ in range(3))+ue(2+tid)+ue(0)
    b += ue(0)+'1'+'10'+ue(0)+'0'+'11'
    b += '1'*((-len(b))%8)
    b += camera_bits(precision, [(False, [(0, [scale, offset, 0, 0])] if camera_available else [])])+'0'+'1'
    return b'\x40\x01'+escape(packed(b))


def control_movie(refinement=None, camera_available=True, camera=None, source_cut=None, colored=False, size=32, direct=False, merge_indices=None, vsp=False, inter_view=True, illumination=None, target_luma=(1,1,0), ic_mv=None, ic_disabled_merge_zero=False, ic_mvp=0, residual_prediction=None, source_luma=(1,1,0), ic_reference=1, dbbp=None, dbbp_mvs=None, dbbp_merge=None, dbbp_weights=None, dbbp_residual=None, dbbp_ac=None, dbbp_log=5, dbbp_cu_log=None, dbbp_cross_mv=None):
    dbbp_cu_log=dbbp_log if dbbp_cu_log is None else dbbp_cu_log
    assert dbbp_cu_log==dbbp_log or (dbbp_cu_log==4 and dbbp_log==5 and dbbp_residual is None and dbbp_mvs is None)
    assert dbbp_cross_mv is None or (dbbp is not None and dbbp_cu_log==5 and dbbp_log==5 and dbbp_mvs is None and dbbp_residual is None)
    assert dbbp_log in [5,6] and size%(1<<dbbp_log)==0
    assert dbbp_log==5 or (dbbp is not None and dbbp_residual is None and source_cut is None)
    assert size>=32 and size%32==0
    assert merge_indices is None or len(merge_indices)==(size//32)**2
    from generate_hevc_layered_sps import sps
    from generate_hevc_layered_pps import se
    from generate_hevc_layered_slices import header_pps
    from generate_hevc_layered_reconstruction import parameter_ids, layer
    from generate_hevc_colour_plane_fixtures import bits, unescape
    from generate_hevc_cqt import reference,texture_amp,banks,merge_index_bins,terminal_zero
    from generate_hevc_3d_sps import payload
    from generate_hevc_texture_iv import target
    from hevc_fixture_mp4 import mux
    base = sps(multilayer=False, independent=True, sub=0, coded_size=(size,size),
               coded_depth=(8,8), coded_chroma=1, pcm=True, pcm_blocks=(5,5),
               pcm_depth=(8,8), amp=True,coding_blocks=(3,dbbp_log))
    raw = bits(unescape(base[2:])); raw = raw[:raw.rfind('1')]
    assert raw.endswith('1010000001')
    base = base[:2]+escape(packed(raw[:-10]+'0'+'1'))
    p0 = header_pps(0,0,False,constrained=direct)
    raw = bits(unescape(p0[2:])); raw = raw[:raw.rfind('1')]
    assert raw.endswith('1010000000010')
    p0 = p0[:2]+escape(packed(raw[:-13]+'0'+'1'))
    scale,offset,precision=(128 if refinement is None else 4,0,0) if camera is None else camera
    params = [vps(scale,camera_available,offset,precision,size),base]+[layer(sps(multilayer=True,sps_id=i,pcm=True,
        pcm_blocks=(5,5),pcm_depth=(8,8),amp=True,coding_blocks=(3,dbbp_log)),id)for i,id in [(1,3),(2,5)]]
    if refinement is not None:
        flags=[False]*14; flags[0]=inter_view; flags[2]=residual_prediction is not None; flags[3]=refinement; flags[4]=vsp; flags[5]=dbbp is not None
        params[3]=layer(sps(multilayer=True,sps_id=2,three_d=payload(flags),pcm=True,
            pcm_blocks=(5,5),pcm_depth=(8,8),amp=True,coding_blocks=(3,dbbp_log)),5)
    params += [p0]+[parameter_ids(header_pps(id,0,dbbp_weights is not None and id==5,constrained=direct),0,[i,i])for i,id in [(1,3),(2,5)]]
    tex,_ = reference(size,dbbp_log,True,chroma_pattern=refinement is None)
    dep,_ = reference(size,dbbp_log,False)
    target_tex=tex;source_current=tex
    if colored:
        def pcm(view):
            encoder,contexts=banks(0);chunks=[];columns=size//32
            ctu=1<<dbbp_log;ctu_columns=size//ctu;locations=[]
            for cy in range(ctu_columns):
                for cx in range(ctu_columns):
                    offsets=[(0,0)]if dbbp_log==5 else[(0,0),(32,0),(0,32),(32,32)]
                    for index,(dx,dy)in enumerate(offsets):
                        locations.append((cx*ctu+dx,cy*ctu+dy,index==0,index==len(offsets)-1,int(cx>0)+int(cy>0)))
            for index,(x,y,start,end,split_context)in enumerate(locations):
                if dbbp_log==6 and start:encoder.decision(contexts[f'split{split_context}'],True)
                encoder.decision(contexts['split0'],False);chunks.append(encoder.finish())
                def luma(xx,yy):
                    value=(3*(x+xx)+5*(y+yy)+17)%256
                    if view:
                        num,den,bias=target_luma if view==1 else source_luma
                        value=min(255,max(0,value*num//den+bias))
                    return value
                chunks.append(bytes(luma(xx,yy)for yy in range(32)for xx in range(32)))
                for component in [0,1]:
                    chunks.append(bytes((13*(x//2+xx)+7*(y//2+yy)+29+83*component+([71,113][component]if view==1 else 0))%256
                        for yy in range(16)for xx in range(16)))
                encoder.low=0;encoder.range=510;encoder.width=9
                if end and index+1<len(locations):terminal_zero(encoder)
            chunks.append(encoder.finish());return b''.join(chunks)
        tex=pcm(0);target_tex=pcm(1);source_current=pcm(2)
    def target_data():
        if dbbp is not None:
            from generate_hevc_cqt import part,context,merge_index_bins
            encoder,c=banks(1);flag=context(encoder,154);ref_context=context(encoder,153);greater=context(encoder,198)
            if dbbp_residual is not None:
                assert dbbp_ac in [None,'vertical','horizontal']
                assert len(dbbp_residual)==3 and any(dbbp_residual)
                assert all(value in [-64,0,64]for value in dbbp_residual)
                transform_split=context(encoder,124);chroma_cbf=context(encoder,149);luma_cbf=context(encoder,111)
                coefficient_contexts=[(context(encoder,x),context(encoder,x),context(encoder,g),context(encoder,107),context(encoder,sig),context(encoder,183))for x,g,sig in [(125,196,155),(108,194,170)]]
            nodes=[];columns=size//(1<<dbbp_log)
            for cy in range(columns):
                for cx in range(columns):
                    if dbbp_cu_log==4:
                        nodes.extend([(int(cx>0)+int(cy>0),False),(None,False),(None,False),(None,True)])
                    else:nodes.append((None,True))
            for cu_index,(root_context,end_ctu) in enumerate(nodes):
                if root_context is not None:encoder.decision(c[f'split{root_context}'],True)
                encoder.decision(c['split0'],False);encoder.decision(c['skip0'],False)
                encoder.decision(c['pred'],False);part(encoder,c,2 if dbbp[0]else 1,False,dbbp_cu_log)
                for index in [0,1]:
                    if index==1 and dbbp_merge is not None:
                        encoder.decision(c['merge'],True)
                        merge_index_bins(encoder,c,dbbp_merge,5)
                        continue
                    encoder.decision(c['merge'],False);encoder.decision(ref_context,bool(index))
                    mv=(0,0) if dbbp_mvs is None else dbbp_mvs[index]
                    if dbbp_cross_mv is not None:
                        # Ref0 is temporal ST, ref1 inter-layer LT: they cannot
                        # supply each other's AMVP. Vertical PU1 matches ref1
                        # above after row zero; horizontal PU1 matches it left
                        # after column zero. PU0's temporal motion stays zero.
                        needs_mvd=cu_index<columns if dbbp[0]else cu_index%columns==0
                        mv=dbbp_cross_mv if index==1 and needs_mvd else(0,0)
                    for value in mv:encoder.decision(c['mvd'],value!=0)
                    for value in mv:
                        if value:encoder.decision(greater,abs(value)>1)
                    for value in mv:
                        if abs(value)>1:
                            remaining=abs(value)-2;k=1
                            while remaining>=1<<k:encoder.bypass(True);remaining-=1<<k;k+=1
                            encoder.bypass(False)
                            for shift in reversed(range(k)):encoder.bypass((remaining>>shift)&1)
                        if value:encoder.bypass(value<0)
                    encoder.decision(c['mvp'],dbbp_mvs is not None and index==1)
                encoder.decision(flag,dbbp[1]);encoder.decision(c['root'],dbbp_residual is not None)
                if dbbp_residual is not None:
                    encoder.decision(transform_split,False)
                    encoder.decision(chroma_cbf,dbbp_residual[1]!=0);encoder.decision(chroma_cbf,dbbp_residual[2]!=0)
                    if dbbp_residual[1] or dbbp_residual[2]:encoder.decision(luma_cbf,dbbp_residual[0]!=0)
                    for component,level in enumerate(dbbp_residual):
                        if not level:continue
                        last_x,last_y,greater_one,greater_two,significant_dc,significant_ac=coefficient_contexts[int(component!=0)]
                        if dbbp_ac=='horizontal':encoder.decision(last_x,True)
                        encoder.decision(last_x,False)
                        if dbbp_ac=='vertical':encoder.decision(last_y,True)
                        encoder.decision(last_y,False)
                        if dbbp_ac=='horizontal':encoder.decision(significant_ac,False)
                        if dbbp_ac is not None:encoder.decision(significant_dc,False)
                        encoder.decision(greater_one,True);encoder.decision(greater_two,True)
                        encoder.bypass(level<0)
                        for bit in '11111111011011':encoder.bypass(int(bit))
                if end_ctu and cu_index+1<len(nodes):terminal_zero(encoder)
            return encoder.finish()
        if size==32 and illumination is None and residual_prediction is None:return target(0 if merge_indices is None else merge_indices[0])
        encoder,contexts=banks(1);columns=size//32
        ic_context=None
        if illumination is not None:
            from generate_hevc_cqt import context
            ic_context=context(encoder,154)
        residual_contexts=None
        if residual_prediction is not None:
            from generate_hevc_cqt import context
            residual_contexts=[context(encoder,v) for v in [162,153,162]]
        residual_weights=[]
        skips=[]
        if ic_mv is not None:
            from generate_hevc_cqt import context
            ref_context=context(encoder,153);greater=context(encoder,198)
        for cy in range(columns):
            for cx in range(columns):
                explicit=ic_mv is not None and (cx,cy)==(1,0)
                encoder.decision(contexts['split0'],False)
                count=int(cx>0 and skips[-1])+int(cy>0 and skips[(cy-1)*columns+cx])
                encoder.decision(contexts[f'skip{count}'],not explicit);skips.append(not explicit)
                if explicit:
                    encoder.decision(contexts['pred'],False);encoder.decision(contexts['part0'],True)
                    encoder.decision(contexts['merge'],False);encoder.decision(ref_context,bool(ic_reference))
                    for value in ic_mv:encoder.decision(contexts['mvd'],value!=0)
                    for value in ic_mv:
                        if value:encoder.decision(greater,abs(value)>1)
                    for value in ic_mv:
                        if abs(value)>1:
                            remaining=abs(value)-2;k=1
                            while remaining>=1<<k:encoder.bypass(True);remaining-=1<<k;k+=1
                            encoder.bypass(False)
                            for shift in reversed(range(k)):encoder.bypass((remaining>>shift)&1)
                        if value:encoder.bypass(value<0)
                    encoder.decision(contexts['mvp'],bool(ic_mvp))
                else:merge_index_bins(encoder,contexts,0 if merge_indices is None else merge_indices[cy*columns+cx],5)
                index=0 if merge_indices is None else merge_indices[cy*columns+cx]
                weight=residual_prediction[cy*columns+cx]if isinstance(residual_prediction,(list,tuple))else residual_prediction
                if residual_contexts is not None:
                    left=bool(cx and residual_weights[-1])
                    encoder.decision(residual_contexts[int(left)],weight!=0)
                    if weight:encoder.decision(residual_contexts[2],weight==2)
                    residual_weights.append(weight)
                flag=illumination[cy*columns+cx]if isinstance(illumination,(list,tuple))else illumination
                if not weight and ic_context is not None and (explicit or index!=0 or not ic_disabled_merge_zero):encoder.decision(ic_context,flag)
                if explicit:encoder.decision(contexts['root'],False)
                if cy*columns+cx+1<columns*columns:terminal_zero(encoder)
        return encoder.finish()
    def header(id,poc):
        if source_cut is not None and id==0 and poc==1:
            b='1'+ue(0)+ue(1)+'1'+f'{poc:08b}'
            b+='0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0'
            b+='00'+'0'+'0'+ue(0) # SAO, default L0, CABAC init, max merge 5
            b+=se(0)+ue(1)+'00000000'+'1';b+='0'*((-len(b))%8)
            return ((1<<9)|1).to_bytes(2,'big')+escape(packed(b)+texture_amp(size,4,top=source_cut,vertical_partition=True))
        if refinement is not None and id==5 and poc==1:
            # Own past picture plus the current-AU texture of view 0.
            b='1'+ue(2)+ue(1)+'1'+f'{poc:08b}'
            b+='0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0'
            b+='1' # inter-layer prediction, sole texture reference layer 0
            if refinement or vsp or dbbp is not None: b+='1' # in_comp_pred_flag: current-AU depth layer 3
            b+='00'+'1'+ue(1)+'0'+'0'
            if dbbp_weights is not None:
                b+=ue(1)+se(0)+'11'+'11'
                for entries in dbbp_weights:
                    b+=se(entries[0][0]-2)+se(entries[0][1])
                    for weight,offset in entries[1:]:b+=se(weight-2)+se(offset-128+64*weight)
            else:b+=('0' if illumination is None else '1'+str(int(ic_disabled_merge_zero)))
            b+=ue(int(inter_view or vsp and camera_available))
            b+=se(0)+ue(1)+'00000000'+'1';b+='0'*((-len(b))%8)
            return ((1<<9)|(5<<3)|1).to_bytes(2,'big')+escape(packed(b)+target_data())
        b = '1'+('0'if poc==0 else'')+ue({0:0,3:1,5:2}[id])+ue(2)+'1'
        if id or poc: b += f'{poc:08b}'
        if poc:
            b += ('0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0') if residual_prediction is not None and id==0 else ('0'+ue(0)+ue(0)+ue(0)+ue(0)+'0')
        if id==5: b += '0' # no active inter-layer texture for this I slice
        if (refinement or vsp or dbbp is not None) and id==5: b+='0' # I-picture in-component prediction disabled
        b += '0'if id==3 else'00'
        b += se(0)+ue(1)+'00000000'+'1'
        b += '0'*((-len(b))%8)
        return (((19 if poc==0 else 1)<<9)|(id<<3)|1).to_bytes(2,'big')+escape(packed(b)+(dep if id==3 else target_tex if id==5 else source_current if poc and id==0 else tex))
    pictures = [[header(id,poc)for id in [0,3,5]]for poc in [0,1]]
    stream = b''.join(b'\x00\x00\x00\x01'+n for n in params+[n for au in pictures for n in au])
    return mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)


if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT/'texture-depth-refinement.vps').write_bytes(vps())
    (OUT/'texture-depth-refinement-control.mp4').write_bytes(control_movie())
    for enabled in [False,True]:
        (OUT/f'texture-depth-refinement-{"on"if enabled else"off"}.mp4').write_bytes(control_movie(enabled))

    (OUT/'texture-depth-refinement-no-camera.mp4').write_bytes(control_movie(True,False))

    import json
    cases=[]
    for precision in [0,2,5]:
        for scale,offset in [(0,0),(1,0),(-1,0),(2,0),(-2,0),(4,0),(-4,0),(128,0),(-128,0),(3,-2),(-3,2)]:
            name=f'texture-depth-refinement-camera-{precision}-{scale}-{offset}.mp4'
            (OUT/name).write_bytes(control_movie(True,camera=(scale,offset,precision)))
            shift=7+precision
            disparity=(scale*254+offset*256+(1<<(shift-1)))>>shift
            cases.append(dict(file=name,precision=precision,scale=scale,offset=offset,disparity=disparity))
    (OUT/'texture-depth-refinement-camera.json').write_text(json.dumps(dict(
        provenance='Owned PCM/CABAC, camera parameters and MP4; no FFmpeg, private source or test-time generation.',
        corner_depths=[37,254,122,83],cases=cases),indent=2)+'\n')

    iv_cases=[]
    for cut in [8,24]:
        for enabled in [False,True]:
            for scale in [0,8,-8,128,-128]:
                name=f'texture-depth-refinement-iv-{cut}-{int(enabled)}-{scale}.mp4'
                (OUT/name).write_bytes(control_movie(enabled,camera=(scale,0,0),source_cut=cut))
                disparity=(scale*254+64)>>7 if enabled else 0
                iv_cases.append(dict(file=name,cut=cut,enabled=enabled,scale=scale,disparity=disparity))
    (OUT/'texture-depth-refinement-iv.json').write_text(json.dumps(dict(
        provenance='Owned vertical AMP source and target IV skip; independent 8x8 lookup oracle in tests.',
        cases=iv_cases),indent=2)+'\n')

    colored_cases=[]
    for precision in [0,2,5]:
        for scale,offset in [(0,0),(1,0),(-1,0),(2,0),(-2,0),(4,0),(-4,0),(6,0),(-6,0),(128,0),(-128,0),(3,-2),(-3,2)]:
            name=f'texture-depth-refinement-colored-{precision}-{scale}-{offset}.mp4'
            (OUT/name).write_bytes(control_movie(True,camera=(scale,offset,precision),colored=True))
            shift=7+precision
            colored_cases.append(dict(file=name,precision=precision,scale=scale,offset=offset,
                disparity=(scale*254+offset*256+(1<<(shift-1)))>>shift))
    (OUT/'texture-depth-refinement-colored.json').write_text(json.dumps(dict(
        provenance='Owned PCM/CABAC and distinct source/target chroma; independent four-tap oracle in tests.',
        cases=colored_cases),indent=2)+'\n')

    colored_iv=[]
    for cut in [8,24]:
        for enabled in [False,True]:
            for scale in [0,8,-8,128,-128]:
                name=f'texture-depth-refinement-colored-iv-{cut}-{int(enabled)}-{scale}.mp4'
                (OUT/name).write_bytes(control_movie(enabled,camera=(scale,0,0),source_cut=cut,colored=True))
                colored_iv.append(dict(file=name,cut=cut,enabled=enabled,scale=scale,
                    disparity=(scale*254+64)>>7 if enabled else 0))
    (OUT/'texture-depth-refinement-colored-iv.json').write_text(json.dumps(dict(
        provenance='Owned vertical AMP source, distinct source/target chroma and target IV skip.',
        cases=colored_iv),indent=2)+'\n')

    # Coded native wide-DI acceptance, including exact retained i64 motion.
    for label,scale in [('positive',2147483647),('negative',-2147483647)]:
        (OUT/f'texture-depth-refinement-wide-{label}.mp4').write_bytes(control_movie(True,camera=(scale,0,0)))
