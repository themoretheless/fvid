"""Original PCM + P/B DC-only inter pictures, sample-domain oracle, offline."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_layered_sps import vps,sps,ue
from generate_hevc_layered_slices import header_pps
from generate_hevc_layered_pps import se
from generate_hevc_depth_tables import header,delta
from generate_hevc_3d_sps import payload
from generate_hevc_wedge_tables import OUT
from generate_hevc_colour_plane_fixtures import bits,unescape,escape,packed
from generate_hevc_layered_reconstruction import configuration,parameter_ids
from generate_he_aac_packet_fixtures import boxes,DEST
from hevc_fixture_mp4 import mux
SCENARIOS=[('unit',1),('positive',10),('negative',-10),('clip',150)]

def terminal_zero(b):
    b.range-=2
    while b.range<256:b.low*=2;b.range*=2;b.width+=1

def context(b,value):
    pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
    index=len(b.contexts);b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
    return index

def reference(size,log):
    b=Encoder(0,26);split=[context(b,v)for v in [139,141,157]];part=context(b,184)
    pieces=[];columns=size//(1<<log);count=columns*columns
    samples=[(x*7+y*11+37)%256 for y in range(size)for x in range(size)]
    for ctu in range(count):
        cx=(ctu%columns)*(1<<log);cy=(ctu//columns)*(1<<log)
        if log==6:b.decision(split[int(cx>0)+int(cy>0)],True)
        for dx,dy in ([(0,0)]if log==5 else[(0,0),(32,0),(0,32),(32,32)]):
            b.decision(part,True)
            pieces.append(b.finish()) # pcm_flag=1 and zero alignment
            pieces.append(bytes(samples[(cy+dy+y)*size+cx+dx+x]for y in range(32)for x in range(32)))
            b.low=0;b.range=510;b.width=9 # retain adapted contexts across PCM
        if ctu+1<count:terminal_zero(b)
    pieces.append(b.finish())
    return b''.join(pieces),samples

def inter(size,log,kind,mode,offset):
    init=1 if kind=='P'else 2;b=Encoder(init,26)
    split=context(b,107)
    skips=[context(b,v)for v in [197,185,201]]
    pred=context(b,149 if init==1 else 134);part=context(b,154);part_horizontal=context(b,139)
    merge=context(b,110 if init==1 else 154)
    mvd=context(b,140 if init==1 else 169);mvp=context(b,168)
    inter0=context(b,95);inter4=context(b,31);root=context(b,79)
    transform_split=context(b,124 if init==1 else 224);cbf=context(b,153)
    last_x=context(b,110 if init==1 else 125);last_y=context(b,110 if init==1 else 125)
    greater=context(b,196);greater2=context(b,107)
    columns=size//(1<<log);count=columns*columns
    for ctu in range(count):
        x=ctu%columns;y=ctu//columns
        if log==6:b.decision(split,False)
        skip=mode=='skip'
        b.decision(skips[(int(x>0)+int(y>0))if skip else 0],skip)
        if not skip:
            b.decision(pred,False);b.decision(part,mode!='partitioned')
            if mode=='partitioned':b.decision(part_horizontal,True)
            for _ in range(2 if mode=='partitioned'else 1):
                b.decision(merge,mode in ['merge','merge-residual'])
                if mode not in ['merge','merge-residual']:
                    if kind=='B':b.decision(inter0,False);b.decision(inter4,False)
                    b.decision(mvd,False);b.decision(mvd,False);b.decision(mvp,False)
            enabled=mode in ['merge','explicit']
            if mode!='partitioned':b.decision(2,enabled)
            if enabled:b.absolute(abs(offset)-1);b.bypass(offset<0)
            elif mode=='merge-residual':
                # Full merge infers rqt_root_cbf=1: ordinary coefficient syntax
                # must remain available when dc_only_flag is explicitly zero.
                for _ in range(4 if log==6 else 1):
                    b.decision(transform_split,False)
                    if log==6:b.decision(cbf,True)
                    b.decision(last_x,False);b.decision(last_y,False)
                    b.decision(greater,True);b.decision(greater2,True);b.bypass(kind=='B')
                    for bit in '11111111011011':b.bypass(int(bit))
            else:b.decision(root,False)
        if ctu+1<count:terminal_zero(b)
    return b.finish()

def depth_nal(poc,entropy):
    kind=19 if poc==0 else 1;stype=2 if poc==0 else 1 if poc==1 else 0
    value=('10'if poc==0 else'1')+ue(1)+ue(stype)+'1'+f'{poc:08b}'
    if poc:
        value+='0'+ue(1)+ue(0)+ue(0)+'1'+ue(0)+ue(0)+'0' # RPS(-1), no LT/MVP
    value+='0' # mono SAO disabled, no opposite-role reference-list flag
    if poc:
        value+='0' # default reference counts
        if stype==0:value+='0' # mvd_l1_zero
        value+='0'+ue(4) # CABAC init false; one merge candidate
    value+=se(0)+ue(1)+'00000000'+'1'
    value+='0'*((-len(value))%8)
    return ((kind<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(value)+entropy)

def generate():
    source=(DEST/'hevc-pcm-reference-rext8.mp4').read_bytes()
    def child(d,t):return next(p for k,p in boxes(d)if k==t)
    stbl=child(child(child(child(child(source,b'moov'),b'trak'),b'mdia'),b'minf'),b'stbl')
    entry=next(boxes(child(stbl,b'stsd')[8:]))[1];params=configuration(child(entry[78:],b'hvcC'))
    sizes=struct.unpack('>3I',child(stbl,b'stsz')[12:]);at=int.from_bytes(child(stbl,b'stco')[8:12],'big')
    base=[]
    for length in sizes:
        packet=source[at:at+length];at+=length
        assert int.from_bytes(packet[:4],'big')==len(packet)-4
        base.append(packet[4:])
    assert [n[0]>>1 for n in base]==[19,1,1]
    # Native metadata for the authored source: I0/B1/B2, all temporal ID zero.
    flags=[False]*14;flags[12]=True
    files={}
    for size,log in [(32,5),(64,6),(128,6)]:
        fmt=dict(chroma=0,separate=False,depth=[8,8],size=[size,size],crop=[0]*4)
        base_fmt=dict(chroma=1,separate=False,depth=[8,8],size=[64,64],crop=[0]*4)
        raw_vps=vps(fmt,depth_layer=True,base_format=base_fmt)[0]
        v=bits(unescape(raw_vps[2:]));ptl=bits(unescape(params[33][0][2:]))[8:104]
        raw_vps=raw_vps[:2]+escape(packed(v[:32]+ptl+v[128:]))
        depth_sps=parameter_ids(sps(multilayer=True,three_d=payload(flags,sub=(2,2)),coding_blocks=(5,log),pcm=True,pcm_blocks=(5,5),pcm_depth=(8,8)),7,[1])
        first,pixels=reference(size,log);files[f'inter-dc-{size}-reference.cabac']=first
        for mode in ['merge','explicit','skip','disabled','partitioned','merge-residual']:
            scenarios=SCENARIOS if mode in ['merge','explicit']else[('identity',0)]
            for label,offset in scenarios:
                p=inter(size,log,'P',mode,offset);b=inter(size,log,'B',mode,-offset)
                stem=f'inter-dc-{size}-{mode}-{label}'
                files[stem+'-P.cabac']=p;files[stem+'-B.cabac']=b
                delta_p=26 if mode=='merge-residual'else offset
                delta_b=-25 if mode=='merge-residual'else -offset
                samples1=[max(0,min(255,v+delta_p))for v in pixels]
                samples2=[max(0,min(255,v+delta_b))for v in samples1]
                files[stem+'.y16']=struct.pack(f'<{size*size*3}H',*(pixels+samples1+samples2))
                pictures=[[base[i],depth_nal(i,data)]for i,data in enumerate([first,p,b])]
                for dlt in [False,True]:
                    tables=header(8,1)+'100'+delta(8,list(range(0,256,8)))if dlt else None
                    for direct in ([False,True]if size==128 else[False]):
                        pps=parameter_ids(header_pps(3,0,False,False,depth_tables=tables,constrained=direct),0,[1,1])
                        parameters=[raw_vps,params[33][0],depth_sps,params[34][0],pps]
                        stream=b''.join(b'\x00\x00\x00\x01'+n for n in parameters+[n for au in pictures for n in au])
                        name=stem+('-dlt'if dlt else'')+('-direct'if direct else'')+'.mp4'
                        files[name]=mux(stream,8,64,64,chroma_format=1,access_units=pictures,multilayer=True)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
