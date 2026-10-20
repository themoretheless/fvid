"""Owned current-AU texture CQT/partition prediction witnesses, fully offline."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_inter_dc import context,terminal_zero
from generate_hevc_layered_sps import vps,sps,ue
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_depth_tables import header,delta
from generate_hevc_3d_sps import payload
from generate_hevc_layered_reconstruction import parameter_ids
from generate_hevc_colour_plane_fixtures import packed,escape
from generate_hevc_wedge_tables import OUT
from hevc_fixture_mp4 import mux
LAYOUTS={'full':(0,0),'hv':(1,2),'top-bottom':(4,5),'left-right':(6,7)}
GEOMETRIES=[(64,5,False),(64,6,True),(128,6,True),(64,6,False),(128,6,False)]

def banks(init):
    b=Encoder(init,26)
    values={'split0':139 if init==0 else 107,'split1':141 if init==0 else 139,'split2':157 if init==0 else 126,
        'part0':184 if init==0 else 154,'part1':139,'part2':154,'part3':154,
        'skip0':197,'skip1':185,'skip2':201,'pred':149 if init==1 else 134,
        'merge':110 if init==1 else 154,'mvd':140 if init==1 else 169,'mvp':168,
        'ip0':95,'ip1':79,'ip4':31,'root':79,'merge_idx':122 if init==1 else 137,'sao_merge':153,'sao_type':[200,185,160][init]}
    return b,{key:context(b,v)for key,v in values.items()}

def reference(size,ctu_log,chroma,flat=False,bias=0,ramp=False,chroma_pattern=False):
    b,c=banks(0);out=[];side=1<<ctu_log;cols=size//side
    y_values=[y+37 if ramp else 128 if flat else ((x*3+y*5+17)if chroma else(x*7+y*11+37+bias))%256 for y in range(size)for x in range(size)]
    for cy in range(cols):
        for cx in range(cols):
            if ctu_log==6:b.decision(c[f'split{int(cx>0)+int(cy>0)}'],True)
            for dx,dy in ([(0,0)]if ctu_log==5 else[(0,0),(32,0),(0,32),(32,32)]):
                b.decision(c['split0'],False) # leaf32 > minimum CU8
                out.append(b.finish()) # PCM is full-partition inferred, no part_mode bin
                x=cx*side+dx;y=cy*side+dy
                out.append(bytes(y_values[(y+yy)*size+x+xx]for yy in range(32)for xx in range(32)))
                if chroma:
                    if chroma_pattern:
                        for component in range(2):
                            out.append(bytes(((x//2+xx)*13+(y//2+yy)*7+29+component*83)%256 for yy in range(16)for xx in range(16)))
                    else:out.append(bytes([128])*512)
                b.low=0;b.range=510;b.width=9
            if cy*cols+cx+1<cols*cols:terminal_zero(b)
    out.append(b.finish())
    return b''.join(out),y_values

def part(b,c,mode,predicted,log):
    # Table I.16; AMP enabled and CU sizes > minimum CU8 throughout.
    b.decision(c['part0'],mode==0)
    if mode==0:return
    if predicted:
        b.decision(c['part1'],mode in (1,2))
        if mode not in (1,2):b.bypass(mode in (5,7))
    else:
        b.decision(c['part1'],mode in (1,4,5))
        b.decision(c['part3'],mode in (1,2))
        if mode not in (1,2):b.bypass(mode in (5,7))

def sao_parameters(b,c,cx,cy,mode):
    if mode is None:return
    if cx>0 or cy>0:
        b.decision(c['sao_merge'],True);return # left, otherwise above
    edge=isinstance(mode,tuple)
    b.decision(c['sao_type'],True);b.bypass(edge)
    offsets=[7,3,-2,-7]if edge else[7,-3,2,-7]
    for value in offsets:
        for _ in range(abs(value)):b.bypass(True)
        if abs(value)<7:b.bypass(False)
    if edge:
        assert mode[0]=='edge'and 0<=mode[1]<4
        for shift in [1,0]:b.bypass((mode[1]>>shift)&1)
    else:
        for value in offsets:b.bypass(value<0)
        for shift in reversed(range(5)):b.bypass((mode>>shift)&1)

def merge_index_bins(b,c,index,maximum):
    assert 0<=index<maximum<=6
    for position in range(maximum-1):
        more=position<index
        if position==0:b.decision(c['merge_idx'],more)
        else:b.bypass(more)
        if not more:break

def predicted(size,ctu_log,split_depth,poc,texture_mode,depth,enabled,mode,start_ctu=0,sao=None,merge_index=0,merge_max=1):
    b,c=banks(1 if poc==1 else 2);side=1<<ctu_log;cols=size//side
    for cy in range(cols):
        for cx in range(cols):
            if cy*cols+cx<start_ctu:continue
            sao_parameters(b,c,cx,cy,sao)
            children=[(0,0,5)]if ctu_log==5 else[(0,0,5),(32,0,5),(0,32,5),(32,32,5)]if split_depth else[(0,0,6)]
            if ctu_log==6:b.decision(c[f'split{(int(cx>0)+int(cy>0))if split_depth else 0}'],split_depth)
            for dx,dy,log in children:
                x=cx*side+dx;y=cy*side+dy;relative_depth=ctu_log-log
                if log==5 and not(depth and enabled):b.decision(c['split0'],False)
                skip=depth and mode=='skip'
                b.decision(c[f'skip{int(x>0)+int(y>0)if skip else 0}'],skip)
                if skip:
                    merge_index_bins(b,c,merge_index,merge_max)
                    continue
                b.decision(c['pred'],False)
                chosen=0 if depth and mode=='dc'else texture_mode
                inferred=depth and enabled and log==5 and texture_mode==0
                idc=0 if not(depth and enabled and log==5)or texture_mode==0 else 1 if texture_mode in (1,4,5)else 2
                if not inferred:part(b,c,chosen,idc,log)
                for _ in range(1 if chosen==0 else 2):
                    b.decision(c['merge'],False)
                    if poc==2:b.decision(c[f'ip{relative_depth}'],False);b.decision(c['ip4'],False)
                    b.decision(c['mvd'],False);b.decision(c['mvd'],False);b.decision(c['mvp'],False)
                if depth and chosen==0:
                    b.decision(2,mode=='dc')
                    if mode=='dc':b.absolute(9);b.bypass(poc==2)
                if not(depth and mode=='dc'):b.decision(c['root'],False)
            if cy*cols+cx+1<cols*cols:terminal_zero(b)
    return b.finish()

def nal(layer,poc,data,enabled,stype=None,first=True,address=0,rps=None,weights=None,sao=False,texture_motion=False,merge_max=1):
    stype=(2 if poc==0 else 1 if poc==1 else 0)if stype is None else stype
    value=str(int(first))+('0'if poc==0 else'')+ue(int(layer>0))
    if not first:value+='0'+f'{address:02b}' # independent segment in a 64x64/CTU32 picture
    value+=ue(stype)+'1'
    if layer>0 or poc:value+=f'{poc:08b}'
    if poc:
        negative,positive=([1],[])if rps is None else rps
        value+='0'+ue(len(negative))+ue(len(positive))
        for distances in [negative,positive]:
            previous=0
            for distance in distances:
                value+=ue(distance-previous-1)+'1';previous=distance
        value+=ue(0)+ue(0)+'0'
    if layer>0:value+=str(int(enabled)) # syntax availability includes the I slice
    value+=str(int(sao))if layer>0 else'00'
    if poc and stype!=2:
        value+='0'+('00'if rps is not None and sum(map(len,rps))>1 else'')+('0'if stype==0 else'')+'0'
        if weights is not None:
            denominator,entries=weights
            value+=ue(denominator)
            for weight,offset in entries:
                value+='1'+se(weight-(1<<denominator))+se(offset)
        assert 1<=merge_max<=5+int(texture_motion and enabled)
        value+=ue(5+int(texture_motion and enabled)-merge_max)
    value+=se(0)+ue(1)+'00000000'+'1';value+='0'*((-len(value))%8)
    return (((19 if poc==0 else 1)<<9)|(layer<<3)|1).to_bytes(2,'big')+escape(packed(value)+data)

def generate():
    files={}
    for size,log,split in GEOMETRIES:
        ref_tex,texture=reference(size,5,True);ref_depth,pixels=reference(size,log,False)
        base_fmt=dict(chroma=1,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
        depth_fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
        raw_vps=vps(depth_fmt,depth_layer=True,base_format=base_fmt)[0]
        base_sps=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
        flags=[False]*14;flags[11]=flags[12]=True
        depth_sps=parameter_ids(sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,log),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),7,[1])
        base_pps=header_pps(0,0,False)
        for layout,(p_part,b_part)in LAYOUTS.items():
            texture_data=[ref_tex,predicted(size,5,False,1,p_part,False,False,'match'),predicted(size,5,False,2,b_part,False,False,'match')]
            for enabled in [False,True]:
                for mode in ['match','dc','skip']:
                    depth_data=[ref_depth,predicted(size,log,split,1,p_part,True,enabled,mode),predicted(size,log,split,2,b_part,True,enabled,mode)]
                    pictures=[[nal(0,i,t,False),nal(3,i,d,enabled)]for i,(t,d)in enumerate(zip(texture_data,depth_data))]
                    stem=f'cqt-{size}-ctu{1<<log}'+('-keep'if log==6 and not split else'')+f'-{layout}-'+('on'if enabled else'off')+'-'+mode
                    wanted=[pixels]
                    for delta in ([10,-10]if mode=='dc'else[0,0]):wanted.append([max(0,min(255,v+delta))for v in wanted[-1]])
                    files[stem+'.y16']=struct.pack(f'<{3*size*size}H',*(v for frame in wanted for v in frame))
                    for dlt in ([False,True]if enabled and mode=='dc'else[False]):
                        tables=header(8,1)+'100'+delta_table()if dlt else None
                        for direct in ([False,True]if size==128 else[False]):
                            depth_pps=parameter_ids(header_pps(3,0,False,depth_tables=tables,constrained=direct),0,[1,1])
                            params=[raw_vps,base_sps,depth_sps,base_pps,depth_pps]
                            stream=b''.join(b'\x00\x00\x00\x01'+n for n in params+[n for au in pictures for n in au])
                            files[stem+('-dlt'if dlt else'')+('-direct'if direct else'')+'.mp4']=mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)
    files.update(mixed_slices())
    files.update(exact_texture_motion())
    files.update(exact_texture_motion(4))
    for size in [32,128]:
        for limit in [False,True]:
            for enabled in [False,True]:
                for direct in ([False,True]if size==128 else[False]):
                    for intra_source in [False,True]:
                        for mode in ['skip','dc']:
                            for cqt in [False,True]:
                                files.update(exact_texture_motion(32767 if limit else 4,active=True,enabled=enabled,size=size,direct=direct,intra_source=intra_source,mode=mode,cqt=cqt))
    for size,direct in [(32,False),(128,False),(128,True)]:
        for mode in ['skip','dc']:
            for denominator,weight,offset in [(0,-3,127),(0,5,-7),(1,3,9),(7,255,-127)]:
                files.update(exact_texture_motion(4,active=True,size=size,direct=direct,
                    mode=mode,weights=(denominator,[(weight,offset)])))
                for sao in [0,8,31]+[('edge',i)for i in range(4)]:
                    files.update(exact_texture_motion(4,active=True,size=size,direct=direct,
                        mode=mode,weights=(denominator,[(weight,offset)]),sao=sao))
    files.update(bidirectional_texture_motion())
    for weights in [(0,[(-3,-7),(5,11)]),(1,[(3,9),(-1,-5)]),(7,[(1,-127),(255,127)])]:
        files.update(bidirectional_texture_motion(weights))
        for size,direct in [(32,False),(128,False),(128,True)]:
            if size!=32:files.update(bidirectional_texture_motion(weights,size=size,direct=direct))
            for sao in [0,8,31]+[('edge',i)for i in range(4)]:
                files.update(bidirectional_texture_motion(weights,sao,size,direct))
    for direct in [False,True]:
        files.update(bidirectional_texture_motion(size=128,direct=direct))
    for direct in [False,True]:
        for deblock in [False,True]:
            files.update(exact_texture_motion(4,active=True,size=128,direct=direct,deblock=deblock))
    for size,direct in [(32,False),(128,False),(128,True)]:
        for mode in ['skip','dc']:
            for position in [0,8,31]+[('edge',i)for i in range(4)]:
                files.update(exact_texture_motion(4,active=True,size=size,direct=direct,mode=mode,sao=position))
    for size,direct in [(32,False),(128,False),(128,True)]:
        for index in [0,5]:
            for intra_source in [False,True]:
                files.update(exact_texture_motion(4,active=True,size=size,direct=direct,intra_source=intra_source,merge_index=index,merge_max=6))
    files.update(tiny_parallel_merge())
    files.update(depth_mvd())
    files.update(depth_mvd(resampling=True))
    files.update(depth_mvd(64))
    files.update(depth_mvd(128))
    return files

def mixed_slices():
    size=64;flags=[False]*14;flags[11]=flags[12]=True
    fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
    base_fmt=dict(fmt,chroma=1)
    raw_vps=vps(fmt,depth_layer=True,base_format=base_fmt)[0]
    base_sps=sps(multilayer=False,independent=True,sub=0,coded_size=(64,64),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    depth_sps=parameter_ids(sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,5),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),7,[1])
    base_pps=header_pps(0,0,False);depth_pps=parameter_ids(header_pps(3,0,False),0,[1,1])
    tex,_=reference(64,5,True);ref,pixels=reference(64,5,False);first_cu,_=reference(32,5,False)
    texture=[tex,predicted(64,5,False,1,0,False,False,'match'),predicted(64,5,False,2,0,False,False,'match')]
    files={'cqt-mixed-I-P.y16':struct.pack('<12288H',*(pixels*3))}
    for enabled in [False,True]:
        tail=predicted(64,5,False,1,0,True,enabled,'match',start_ctu=1)
        final=predicted(64,5,False,2,0,True,enabled,'match')
        pictures=[[nal(0,0,texture[0],False),nal(3,0,ref,enabled)],
            [nal(0,1,texture[1],False),nal(3,1,first_cu,enabled,stype=2),nal(3,1,tail,enabled,first=False,address=1)],
            [nal(0,2,texture[2],False),nal(3,2,final,enabled)]]
        stream=b''.join(b'\x00\x00\x00\x01'+n for n in [raw_vps,base_sps,depth_sps,base_pps,depth_pps]+[n for au in pictures for n in au])
        files['cqt-mixed-I-P-'+('on'if enabled else'off')+'.mp4']=mux(stream,8,64,64,chroma_format=1,access_units=pictures,multilayer=True)
    return files

def texture_amp(size,magnitude,poc=1,top=8,vertical_partition=False):
    assert top in (8,24)
    amp_top=top
    b,c=banks(1 if poc==1 else 2);c['mvd1']=context(b,198);grid={}
    def sample(x,y):return grid.get((x//4,y//4))if x>=0 and y>=0 and x<size and y<size else None
    for cy in range(size//32):
        for cx in range(size//32):
            b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
            part(b,c,(6 if amp_top==8 else 7)if vertical_partition else(4 if amp_top==8 else 5),False,5)
            rectangles=([(0,0,amp_top,32),(amp_top,0,32-amp_top,32)]if vertical_partition
                else[(0,0,32,amp_top),(0,amp_top,32,32-amp_top)])
            for index,(dx,dy,w,h)in enumerate(rectangles):
                x=cx*32+dx;y=cy*32+dy
                left=[sample(x-1,y+h),sample(x-1,y+h-1)]
                top=[sample(x+w,y-1),sample(x+w-1,y-1),sample(x-1,y-1)]
                lv=next((v for v in left if v is not None),None);tv=next((v for v in top if v is not None),None)
                if not any(v is not None for v in left)and tv is not None:lv=tv
                predictor=lv if lv is not None else tv if tv is not None else(0,0)
                wanted=(0,magnitude if index==0 else-magnitude)
                difference=tuple((v-p+32768)%65536-32768 for v,p in zip(wanted,predictor))
                b.decision(c['merge'],False)
                if poc==2:b.decision(c['ip0'],False);b.decision(c['ip4'],False)
                for v in difference:b.decision(c['mvd'],v!=0)
                for v in difference:
                    if v:b.decision(c['mvd1'],abs(v)>1)
                for v in difference:
                    if not v:continue
                    if abs(v)>1:
                        value=abs(v)-2;k=1
                        while value>=(1<<k):b.bypass(True);value-=1<<k;k+=1
                        b.bypass(False)
                        for shift in reversed(range(k)):b.bypass((value>>shift)&1)
                    b.bypass(v<0)
                b.decision(c['mvp'],False)
                for yy in range(y//4,(y+h)//4):
                    for xx in range(x//4,(x+w)//4):grid[xx,yy]=wanted
            b.decision(c['root'],False)
            if cy*(size//32)+cx+1<(size//32)**2:terminal_zero(b)
    return b.finish()

def texture_amp_bi(size,top=8,vertical_partition=False,motion_amplitudes=(4,-8),active_lists=(True,True)):
    """Explicit B motion with different POCs in L0/L1, independently modelled AMVP."""
    assert top in (8,24)
    amp_top=top
    b,c=banks(2);c['mvd1']=context(b,198);grids=[{},{}]
    for cy in range(size//32):
        for cx in range(size//32):
            b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
            part(b,c,(6 if amp_top==8 else 7)if vertical_partition else(4 if amp_top==8 else 5),False,5)
            rectangles=([(0,0,amp_top,32),(amp_top,0,32-amp_top,32)]if vertical_partition
                else[(0,0,32,amp_top),(0,amp_top,32,32-amp_top)])
            for index,(dx,dy,w,h)in enumerate(rectangles):
                x=cx*32+dx;y=cy*32+dy
                b.decision(c['merge'],False);b.decision(c['ip0'],all(active_lists))
                if not all(active_lists):b.decision(c['ip4'],active_lists[1])
                for list,amplitude in enumerate(motion_amplitudes):
                    if not active_lists[list]:continue
                    grid=grids[list]
                    def sample(x,y):return grid.get((x//4,y//4))if 0<=x<size and 0<=y<size else None
                    left=[sample(x-1,y+h),sample(x-1,y+h-1)]
                    top=[sample(x+w,y-1),sample(x+w-1,y-1),sample(x-1,y-1)]
                    predictor=next((v for v in left+top if v is not None),(0,0))
                    wanted=(0,amplitude if index==0 else-amplitude)
                    difference=tuple((v-p+32768)%65536-32768 for v,p in zip(wanted,predictor))
                    for v in difference:b.decision(c['mvd'],v!=0)
                    for v in difference:
                        if v:b.decision(c['mvd1'],abs(v)>1)
                    for v in difference:
                        if not v:continue
                        if abs(v)>1:
                            value=abs(v)-2;k=1
                            while value>=1<<k:b.bypass(True);value-=1<<k;k+=1
                            b.bypass(False)
                            for shift in reversed(range(k)):b.bypass((value>>shift)&1)
                        b.bypass(v<0)
                    b.decision(c['mvp'],False)
                    for yy in range(y//4,(y+h)//4):
                        for xx in range(x//4,(x+w)//4):grid[xx,yy]=wanted
            b.decision(c['root'],False)
            if cy*(size//32)+cx+1<(size//32)**2:terminal_zero(b)
    return b.finish()

def depth_texture_prediction(size,poc,enabled,mode,intra_source,cqt=True,sao=None,merge_index=0,merge_max=1):
    enabled=enabled and cqt
    if mode=='skip':return predicted(size,5,False,poc,0,True,enabled,'skip',sao=sao,merge_index=merge_index,merge_max=merge_max)
    assert mode=='dc'
    b,c=banks(1 if poc==1 else 2)
    for index in range((size//32)**2):
        sao_parameters(b,c,index%(size//32),index//(size//32),sao)
        if not enabled:b.decision(c['split0'],False)
        b.decision(c['skip0'],False);b.decision(c['pred'],False)
        if not(enabled and intra_source):b.decision(c['part0'],True)
        b.decision(c['merge'],True)
        merge_index_bins(b,c,merge_index,merge_max)
        b.decision(2,True);b.absolute(9);b.bypass(poc==2)
        if index+1<(size//32)**2:terminal_zero(b)
    return b.finish()

def exact_texture_motion(magnitude=1,active=False,enabled=True,size=32,direct=False,intra_source=False,mode='skip',cqt=True,weights=None,deblock=None,sao=None,merge_index=0,merge_max=1,textured_source=False):
    assert magnitude in (1,4,32767)
    # Each texture CU has an eight-row AMP PB and a 24-row PB with
    # opposite quarter-sample motion; a 16x16 map loses the row-8 boundary.
    flags=[False]*14;flags[11]=cqt;flags[12]=True;flags[8]=active
    fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
    profile=(8,sum(1<<bit for bit in [43,42,41,40,39,35]))if active else None # I.11.1.1
    ordering=[([16,16],2+tid,0)for tid in range(3)]if active else None
    raw_vps=vps(fmt,depth_layer=True,base_format=dict(fmt,chroma=1),
        enhancement_profile=profile,ordering_rows=ordering)[0]
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    depth=parameter_ids(sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,5),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),7,[1])
    base_pps=header_pps(0,0,False)
    if active:
        from generate_hevc_colour_plane_fixtures import bits,unescape
        raw=bits(unescape(base[2:]));raw=raw[:raw.rfind('1')]
        assert raw.endswith('1010000001')
        base=base[:2]+escape(packed(raw[:-10]+'0'+'1'))
        raw=bits(unescape(base_pps[2:]));raw=raw[:raw.rfind('1')]
        assert raw.endswith('1010000000010')
        base_pps=base_pps[:2]+escape(packed(raw[:-13]+'0'+'1'))
    pps=[base_pps,parameter_ids(header_pps(3,0,weights is not None,constrained=direct,deblock=bool(deblock)),0,[1,1])]
    tex,_=reference(size,5,True,flat=not textured_source,chroma_pattern=textured_source);dep,_=reference(size,5,False,ramp=deblock is not None)
    if intra_source:
        first_texture=tex
    elif size==32 and magnitude in (1,4):
        b,c=banks(1);c['mvd1']=context(b,198)
        b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
        part(b,c,4,False,5)
        for index in range(2):
            b.decision(c['merge'],False)
            b.decision(c['mvd'],False);b.decision(c['mvd'],True)
            b.decision(c['mvd1'],magnitude>1)
            if magnitude==4:
                for bit in [True,False,False,False]:b.bypass(bit) # bypass Exp-Golomb order 1, abs-2=2
            b.bypass(index==1)
            b.decision(c['mvp'],index==1) # second predictor is the zero pad
        b.decision(c['root'],False)
        first_texture=b.finish()
    else:
        first_texture=texture_amp(size,magnitude)
    pictures=[[nal(0,0,tex,False),nal(3,0,dep,active and enabled)],
        [nal(0,1,first_texture,False,stype=2 if intra_source else None),nal(3,1,depth_texture_prediction(size,1,enabled,mode,intra_source,cqt,sao=sao,merge_index=merge_index,merge_max=merge_max)if active else predicted(size,5,False,1,0,True,False,'match'),active and enabled,weights=weights,sao=sao is not None,texture_motion=active,merge_max=merge_max)]]
    if active and weights is None and deblock is None:
        pictures.append([nal(0,2,tex if intra_source else texture_amp(size,magnitude,poc=2),False,stype=2 if intra_source else None),nal(3,2,depth_texture_prediction(size,2,enabled,mode,intra_source,cqt,sao=sao,merge_index=merge_index,merge_max=merge_max),enabled,sao=sao is not None,texture_motion=active,merge_max=merge_max)])
    stream=b''.join(b'\x00\x00\x00\x01'+n for n in [raw_vps,base,depth]+pps+[n for au in pictures for n in au])
    name=('texture-motion'+(''if size==32 else'-'+str(size))+('-limit'if magnitude==32767 else'')+'-'+('on'if enabled else'off')+('-dc'if mode=='dc'else'')+('-intra-source'if intra_source else'')+('-no-cqt'if not cqt else'')+('-direct'if direct else'')+'.mp4')if active else('cqt-exact-texture-motion.mp4'if magnitude==1 else'cqt-integer-texture-motion.mp4')
    if merge_max!=1:name=name[:-4]+f"-merge{merge_max}-{merge_index}.mp4"
    if sao is not None:
        label=f'edge-{sao[1]}'if isinstance(sao,tuple)else str(sao)
        name=name[:-4]+f"-sao-{label}.mp4"
    if deblock is not None:name=name[:-4]+f"-deblock-{'on'if deblock else'off'}.mp4"
    if weights is not None:
        denominator,[(weight,offset)]=weights
        name=name[:-4]+f'-weighted-{denominator}-{weight}-{offset}.mp4'
    return {name:mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)}

def tiny_parallel_merge():
    size=32;flags=[False]*14;flags[11]=flags[12]=True
    fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
    raw_vps=vps(fmt,depth_layer=True,base_format=dict(fmt,chroma=1))[0]
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    depth=parameter_ids(sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,5),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),7,[1])
    tex,_=reference(size,5,True,flat=True);dep,_=reference(size,5,False)
    texture=[tex,predicted(size,5,False,1,0,True,False,'skip'),predicted(size,5,False,2,0,True,False,'skip')]
    first_p=predicted(size,5,False,1,0,True,False,'skip')
    files={}
    for horizontal in [False,True]:
        b,c=banks(2);depths={}
        def tree(x,y,log,level):
            if log>3:
                ctx=int(depths.get((x//8-1,y//8),-1)>level)+int(depths.get((x//8,y//8-1),-1)>level)
                b.decision(c[f'split{ctx}'],True)
                half=1<<(log-1)
                for dx,dy in [(0,0),(half,0),(0,half),(half,half)]:tree(x+dx,y+dy,log-1,level+1)
            else:
                b.decision(c['skip0'],False);b.decision(c['pred'],False)
                b.decision(c['part0'],False);b.decision(c['part1'],horizontal)
                for _ in range(2):b.decision(c['merge'],True)
                b.decision(c['root'],False)
                depths[x//8,y//8]=level
        tree(0,0,5,0)
        for parallel in [2,3]:
            pps=[header_pps(0,0,False),parameter_ids(header_pps(3,0,False,parallel_merge=parallel),0,[1,1])]
            pictures=[[nal(0,i,t,False),nal(3,i,d,False)]for i,(t,d)in enumerate(zip(texture,[dep,first_p,b.finish()]))]
            stream=b''.join(b'\x00\x00\x00\x01'+n for n in [raw_vps,base,depth]+pps+[n for au in pictures for n in au])
            name=f'tiny-merge-{"horizontal"if horizontal else"vertical"}-parallel{parallel}.mp4'
            files[name]=mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)
    return files

def depth_mvd(size=32,resampling=False):
    flags=[False]*14;flags[11]=flags[12]=True
    fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
    raw_vps=vps(fmt,depth_layer=True,base_format=dict(fmt,chroma=1))[0]
    base=sps(multilayer=False,independent=True,sub=0,coded_size=(size,size),coded_depth=(8,8),coded_chroma=1,pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True)
    depth=parameter_ids(sps(multilayer=True,three_d=payload(flags),coding_blocks=(3,5),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8),amp=True),7,[1])
    pps=[header_pps(0,0,False),parameter_ids(header_pps(3,0,False),0,[1,1])]
    tex,_=reference(size,5,True,flat=True);dep,pixels=reference(size,5,False)
    texture=[tex,predicted(size,5,False,1,0,True,False,'skip'),predicted(size,5,False,2,0,True,False,'skip')]
    def coded(poc,mv):
        b,c=banks(1 if poc==1 else 2);c['mvd1']=context(b,198)
        for index in range((size//32)**2):
            difference=mv if index==0 else(0,0)
            b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
            b.decision(c['part0'],True);b.decision(c['merge'],False)
            if poc==2:b.decision(c['ip0'],False);b.decision(c['ip4'],False)
            for v in difference:b.decision(c['mvd'],v!=0)
            for v in difference:
                if v:b.decision(c['mvd1'],abs(v)>1)
            for v in difference:
                if not v:continue
                if abs(v)>1:
                    value=abs(v)-2;k=1
                    while value>=(1<<k):b.bypass(True);value-=1<<k;k+=1
                    b.bypass(False)
                    for shift in reversed(range(k)):b.bypass((value>>shift)&1)
                b.bypass(v<0)
            b.decision(c['mvp'],False)
            b.decision(2,False);b.decision(c['root'],False)
            if index+1<(size//32)**2:terminal_zero(b)
        return b.finish()
    files={}
    cases=[('resampling',(0,1))]if resampling else[('zero',(0,0)),('right',(1,0)),('left',(-1,0)),('down',(0,1)),('up',(0,-1)),('positive-limit',(0,32767)),('negative-limit',(0,-32768))]
    for label,mv in cases:
        b_mv=(32767,-32768)if resampling else(0,0)
        # B uses a zero predictor; the resampling witness codes both signed extrema.
        pictures=[[nal(0,i,t,False),nal(3,i,d,False)]for i,(t,d)in enumerate(zip(texture,[dep,coded(1,mv),coded(2,b_mv)]))]
        stream=b''.join(b'\x00\x00\x00\x01'+n for n in [raw_vps,base,depth]+pps+[n for au in pictures for n in au])
        stem='depth-mvd-'+(''if size==32 else str(size)+'-')+label
        files[stem+'.mp4']=mux(stream,8,size,size,chroma_format=1,access_units=pictures,multilayer=True)
        shifted=[pixels[min(size-1,max(0,y+mv[1]))*size+min(size-1,max(0,x+mv[0]))]for y in range(size)for x in range(size)]
        second=[shifted[min(size-1,max(0,y+b_mv[1]))*size+min(size-1,max(0,x+b_mv[0]))]for y in range(size)for x in range(size)]
        files[stem+'.y16']=struct.pack(f'<{3*size*size}H',*(pixels+shifted+second))
    return files

def delta_table():return delta(8,list(range(0,256,8)))


def bidirectional_texture_motion(weights=None,sao=None,size=32,direct=False):
    """I0/I2/B1 decode order, distinct depth references and explicit L0/L1 MVs."""
    # Reuse the parameter sets of a valid 3D Main T witness.
    ordinary=next(iter(exact_texture_motion(4,active=True,size=size,direct=direct).values()))
    # Build parameters directly from its hvcC arrays, without codec dependencies.
    start=ordinary.index(b'hvcC')+4
    data=ordinary[start:];position=23;parameters=[]
    for _ in range(data[22]):
        count=int.from_bytes(data[position+1:position+3],'big');position+=3
        for _ in range(count):
            length=int.from_bytes(data[position:position+2],'big');position+=2
            parameters.append(data[position:position+length]);position+=length
    if weights is not None:
        # Replace only the depth PPS; texture remains ordinary unweighted bi.
        parameters=[n for n in parameters if not (((int.from_bytes(n[:2],'big')>>9)&63)==34 and ((int.from_bytes(n[:2],'big')>>3)&63)==3)]
        parameters.append(parameter_ids(header_pps(3,0,True,constrained=direct),0,[1,1]))
    tex,_=reference(size,5,True,flat=True)
    d0,_=reference(size,5,False);d2,_=reference(size,5,False,bias=37 if sao is not None else 97)
    b,c=banks(2);c['mvd1']=context(b,198)
    b.decision(c['split0'],False);b.decision(c['skip0'],False);b.decision(c['pred'],False)
    part(b,c,4,False,5)
    for index in range(2):
        b.decision(c['merge'],False);b.decision(c['ip0'],True) # bi, no ip4
        for motion in ([4,-8]if index==0 else[-4,8]):
            b.decision(c['mvd'],False);b.decision(c['mvd'],True)
            b.decision(c['mvd1'],True)
            value=abs(motion)-2;k=1
            while value>=1<<k:b.bypass(True);value-=1<<k;k+=1
            b.bypass(False)
            for shift in reversed(range(k)):b.bypass((value>>shift)&1)
            b.bypass(motion<0)
            b.decision(c['mvp'],index==1) # zero padded AMVP for bottom PB
    b.decision(c['root'],False)
    pictures=[
        [nal(0,0,tex,False),nal(3,0,d0,True)],
        [nal(0,2,tex,False,stype=2,rps=([2],[])),nal(3,2,d2,True,stype=2,rps=([2],[]))],
        [nal(0,1,b.finish()if size==32 else texture_amp_bi(size),False,stype=0,rps=([1],[1])),
         nal(3,1,depth_texture_prediction(size,2,True,'skip',False,sao=sao),True,stype=0,rps=([1],[1]),weights=weights,sao=sao is not None,texture_motion=True)]]
    stream=b''.join(b'\x00\x00\x00\x01'+n for n in parameters+[n for au in pictures for n in au])
    label=''if sao is None else'-sao-'+(f'edge-{sao[1]}'if isinstance(sao,tuple)else str(sao))
    name='texture-motion-bi-distinct'+(''if size==32 else f'-{size}')+('-direct'if direct else'')+(''if weights is None else'-weighted-'+str(weights[0]))+label+'.mp4'
    return {name:mux(stream,8,size,size,chroma_format=1,
        access_units=pictures,multilayer=True,presentation_order=[0,2,1])}

if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
