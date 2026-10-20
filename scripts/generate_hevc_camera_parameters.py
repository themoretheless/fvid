"""Owned Annex I VPS camera metadata and two-frame regression movies, offline."""
import json,struct
from generate_hevc_inter_view_scaling import vps,generate_case
from generate_hevc_colour_plane_fixtures import bits,packed,unescape,escape
from generate_hevc_layered_sps import ue
from generate_hevc_layered_pps import se
from generate_he_aac_packet_fixtures import boxes
from hevc_fixture_mp4 import box
from generate_hevc_wedge_tables import OUT

def camera_bits(precision,views):
    value=ue(precision)
    for in_header,pairs in views:
        value+=f'{len(pairs):06b}'
        if pairs:
            value+=str(int(in_header))
            for reference,parameters in pairs:
                value+=ue(reference)
                if not in_header:value+=''.join(se(v)for v in parameters)
    return value

def camera_nal(precision,views,truncate=False,original=None):
    if original is None:original=vps((100,102,108))
    body=bits(unescape(original[2:]));body=body[:body.rfind('1')]
    assert body.endswith('1'+'0'*13)
    body=body[:-14]
    value=camera_bits(precision,views)
    if truncate:value=value[:-4]
    return original[:2]+escape(packed(body+value+'0'+'1'))

def replace_vps(movie,replacement):
    roots=list(boxes(movie));assert [t for t,_ in roots]==[b'ftyp',b'mdat',b'moov']
    def config(data):
        out=bytearray(data[:23]);at=23
        for _ in range(data[22]):
            header=data[at:at+3];at+=3;out+=header
            for _ in range(int.from_bytes(header[1:],'big')):
                count=int.from_bytes(data[at:at+2],'big');at+=2
                nal=data[at:at+count];at+=count
                if header[0]&63==32:nal=replacement
                out+=struct.pack('>H',len(nal))+nal
        assert at==len(data)
        return bytes(out)
    def rewrite(tag,data):
        if tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
            data=b''.join(rewrite(t,p)for t,p in boxes(data))
        elif tag==b'stsd':
            data=data[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v)if k==b'hvcC'else v)for k,v in boxes(p[78:])))for t,p in boxes(data[8:]))
        return box(tag,data)
    return b''.join(rewrite(t,p)for t,p in roots)

def generate():
    files={};cases=[]
    empty=[(False,[]),(False,[])]
    static=[(False,[(0,[3,-4,5,-6])]),(False,[(0,[128,-64,7,-8]),(1,[-11,12,-13,14])])]
    specifications=[
        ('none',0,empty,None,True),
        ('static',5,static,None,True),
        ('deferred',2,[(True,[(0,None)]),(True,[(0,None),(1,None)])],None,False),
        ('sparse',1,[(False,[(65535,[2147483647,-2147483647,0,1])]),(False,[])],None,False),
        ('bad-precision',6,empty,'HEVC VPS camera precision exceeds range',True),
        ('duplicate',0,[(False,[]),(False,[(0,[1,2,3,4]),(0,[5,6,7,8])])],'duplicate HEVC VPS camera reference',True),
        ('bad-reference',0,[(False,[(65536,[1,2,3,4])]),(False,[])],'HEVC VPS camera reference exceeds range',True),
        ('truncated',0,static,'truncated HEVC VPS 3D extension',True),
    ]
    movie=generate_case('',(100,102,108),4)['inter-view-temporal-scaling-on.mp4']
    for name,precision,views,error,video in specifications:
        nal=camera_nal(precision,views,truncate=name=='truncated')
        files[f'camera-parameters-{name}.vps']=nal
        raw=camera_bits(precision,views)
        if name=='truncated':raw=raw[:-4]
        files[f'camera-parameters-{name}.bin']=packed(raw+'1')
        if video:files[f'camera-parameters-{name}.mp4']=replace_vps(movie,nal)
        cases.append(dict(name=name,precision=precision,views=views,error=error,video=video))
    # A profile ID in another namespace does not select Annex I grammar.
    raw=bits(unescape(camera_nal(6,empty)[2:]));raw=raw[:raw.rfind('1')]
    constraints=sum(1<<bit for bit in [43,42,41,40,39,35])
    marker=f'{8:08b}'+'0'*32+f'{constraints:048b}'
    assert raw.count(marker)==1
    raw=raw.replace(marker,'01'+marker[2:],1)
    files['camera-parameters-unrelated-namespace.vps']=b'\x40\x01'+escape(packed(raw+'1'))
    files['camera-parameters-manifest.json']=(json.dumps(dict(provenance='Own synthetic three-view PCM/CABAC movie and explicit camera VPS fields. No private data, external codec or network; tests read pre-generated files.',cases=cases,opaque_namespace_control='camera-parameters-unrelated-namespace.vps'),indent=2)+'\n').encode()
    return files

if __name__=='__main__':
    for name,data in generate().items():(OUT/name).write_bytes(data)
