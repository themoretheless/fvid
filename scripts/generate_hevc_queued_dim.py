"""Real 128x96 queued DIM pictures, independent oracle, synthetic PCM texture."""
import struct
from generate_hevc_depth_dc import Encoder
from generate_hevc_nxn_dim import prediction
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

def generate():
    source=(DEST/'hevc-pcm-parallel-rext8.mp4').read_bytes()
    def child(d,t):return next(p for k,p in boxes(d)if k==t)
    stbl=child(child(child(child(child(source,b'moov'),b'trak'),b'mdia'),b'minf'),b'stbl')
    entry=next(boxes(child(stbl,b'stsd')[8:]))[1]
    params=configuration(child(entry[78:],b'hvcC'))
    length=int.from_bytes(child(stbl,b'stsz')[12:16],'big');offset=int.from_bytes(child(stbl,b'stco')[8:12],'big')
    packet=source[offset:offset+length]
    assert int.from_bytes(packet[:4],'big')==len(packet)-4
    base=packet[4:];rbsp=unescape(base[2:])
    # Owned Rust metadata: first IDR, no extra/output bits, POC width8,
    # three SPS short-term sets, no long-term syntax, temporal MVP enabled.
    assert base[:2]==bytes.fromhex('2601') and rbsp[:3]==bytes.fromhex('af06b8')
    original=bits(rbsp[:3]);align=original.rfind('1');assert original[:6]=='101011'
    prefix=original[:6]+'00000000'+'00'+ue(0)+ue(0)+'0'+original[6:align]+'1'
    prefix+='0'*((-len(prefix))%8)
    base=bytes.fromhex('2a01')+escape(packed(prefix)+rbsp[3:]) # CRA, unchanged PCM/entropy
    fmt=dict(chroma=0,separate=False,depth=[8,8],size=[128,96],crop=[0]*4)
    base_fmt=dict(chroma=1,separate=False,depth=[8,8],size=[128,96],crop=[0]*4)
    raw_vps=vps(fmt,depth_layer=True,base_format=base_fmt)[0]
    value=bits(unescape(raw_vps[2:]));ptl=bits(unescape(params[33][0][2:]))[8:104]
    raw_vps=raw_vps[:2]+escape(packed(value[:32]+ptl+value[128:]))
    flags=[False]*14;flags[9]=flags[10]=True
    depth_sps=parameter_ids(sps(multilayer=True,three_d=payload(flags,sub=(2,2)),coding_blocks=(5,5)),7,[1])
    texture=(DEST/'hevc-pcm-parallel-rext8.yuv').read_bytes()[:128*96]
    files={}
    for partition in ['nxn','full','dc']:
        for residual,label in [(False,'zero')]+([]if partition=='dc'else[(True,'mixed')]):
            b=Encoder(0,26)
            for value in [184,154,154,138 if partition=='nxn'else 153,111 if partition=='nxn'else 141,125 if partition=='nxn'else 111,125 if partition=='nxn'else 111,92,138]:
                pre=max(1,min(126,(((value>>4)*5-45)*26>>4)+(value&15)*8-16))
                b.contexts.append([63-pre if pre<=63 else pre-64,int(pre>63)])
            # slots: Part3, NoDim4, Mode5, Split16=6, CBF7, LastX/Y8/9, Greater10/11.
            for ctu in range(12):
                b.decision(3,partition!='nxn')
                modes='wcwc'if partition=='nxn'else'w'if ctu%2==0 else'c'
                for mode in modes:
                    b.decision(4,False);b.decision(5,mode=='c')
                    if mode=='w':
                        for _ in range(9):b.bypass(False)
                if partition!='nxn':b.decision(2,partition=='dc')
                if partition=='dc':b.decision(0,True)
                for _ in modes:
                    for negative in [True,False]:b.absolute(10);b.bypass(negative)
                levels=([0,64,-64,64]if residual else[0]*4)
                if partition!='nxn':levels=[levels[ctu%4]]
                if partition=='dc':levels=[]
                for level in levels:
                    b.decision(6,False);b.decision(7,level!=0)
                    if level:
                        b.decision(8,False);b.decision(9,False)
                        b.decision(10,True);b.decision(11,True);b.bypass(level<0)
                        for bit in '11111111011011':b.bypass(int(bit))
                if ctu<11:
                    b.range-=2
                    while b.range<256:b.low*=2;b.range*=2;b.width+=1
            entropy=b.finish();family='queued-dim'if partition=='nxn'else f'queued-dim-{partition}'
            files[f'{family}-{label}.cabac']=entropy
            prefix='10'+ue(1)+ue(2)+'1'+'00000000'+'0'+ue(0)+ue(0)+ue(0)+ue(0)+'0'+'1'+'0'+se(0)+ue(1)+'00000000'+'1'
            prefix+='0'*((-len(prefix))%8)
            depth=((21<<9)|(3<<3)|1).to_bytes(2,'big')+escape(packed(prefix)+entropy)
            for dlt in [False,True]:
                tables=header(8,1)+'100'+delta(8,list(range(0,256,8)))if dlt else None
                depth_pps=parameter_ids(header_pps(3,0,False,False,depth_tables=tables),0,[1,1])
                parameters=[raw_vps,params[33][0],depth_sps,params[34][0],depth_pps]
                pictures=[base,depth]
                stream=b''.join(b'\x00\x00\x00\x01'+n for n in parameters+pictures)
                stem=f'{family}-{label}'+('-dlt'if dlt else'')
                files[stem+'.mp4']=mux(stream,8,128,96,chroma_format=1,access_units=[pictures],multilayer=True)
                direct_pps=parameter_ids(header_pps(3,0,False,False,depth_tables=tables,constrained=True),0,[1,1])
                direct_stream=b''.join(b'\x00\x00\x00\x01'+n for n in parameters[:-1]+[direct_pps]+pictures)
                files[stem+'-direct.mp4']=mux(direct_stream,8,128,96,chroma_format=1,access_units=[pictures],multilayer=True)

                image=[None]*(128*96)
                for ctu in range(12):
                    cx=(ctu%4)*32;cy=(ctu//4)*32
                    modes='wcwc'if partition=='nxn'else'w'if ctu%2==0 else'c'
                    n=16 if partition=='nxn'else 32
                    for i,mode in enumerate(modes):
                        x=cx+(i%2)*n;y=cy+(i//2)*n
                        values=prediction(image,texture,mode,x,y,dlt,n,(128,96),texture_stride=128)
                        levels=([0,64,-64,64]if residual else[0]*4)
                        level=levels[i if partition=='nxn'else ctu%4]
                        shift=(0 if not level else 51 if level>0 else -51)if n==16 else (0 if not level else 26 if level>0 else -25)
                        for yy in range(n):
                            for xx in range(n):image[(y+yy)*128+x+xx]=max(0,min(255,values[yy*n+xx]+shift))
                files[stem+'.y16']=struct.pack('<12288H',*image)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
