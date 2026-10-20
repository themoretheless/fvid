"""Own in-band unused SPS3D updates over unchanged synthetic I/P/B pictures."""
import struct
from generate_he_aac_packet_fixtures import DEST,boxes
from generate_hevc_extension_tail_fixtures import extended
from generate_hevc_3d_sps import payload
from hevc_fixture_mp4 import box
OUT=DEST/'hevc-3d-sps'
def generate():
    files={}
    def child(data,tag):return next(p for t,p in boxes(data)if t==tag)
    for name in ['main-ipb','main10-ipb']:
        seed=(DEST.parent/'hevc'/f'{name}.mp4').read_bytes();roots=list(boxes(seed));assert roots[-1][0]==b'moov'
        stbl=child(child(child(child(dict(roots)[b'moov'],b'trak'),b'mdia'),b'minf'),b'stbl')
        sizes=child(stbl,b'stsz');constant,count=struct.unpack_from('>II',sizes,4);assert constant==0
        lengths=struct.unpack('>'+str(count)+'I',sizes[12:]);chunks=child(stbl,b'stco');assert int.from_bytes(chunks[4:8],'big')==1
        at=int.from_bytes(chunks[8:12],'big');original=[]
        for n in lengths:original.append(seed[at:at+n]);at+=n
        entry=next(boxes(child(stbl,b'stsd')[8:]))[1];hvcc=child(entry[78:],b'hvcC');at=23;sps=None
        for _ in range(hvcc[22]):
            kind=hvcc[at]&63;n=int.from_bytes(hvcc[at+1:at+3],'big');at+=3
            for _ in range(n):
                size=int.from_bytes(hvcc[at:at+2],'big');at+=2;nal=hvcc[at:at+size];at+=size
                if kind==33:sps=nal
        assert sps is not None
        depth=[False]*14;depth[9]=True
        texture=[False]*14;texture[0]=True
        updates=[extended(sps,0,payload(),extension_flags='00100000'),extended(sps,0,payload(depth),extension_flags='00100000'),extended(sps,0,payload(texture),extension_flags='00100000')]
        for mode in ['inactive','depth-only','toggle','active-texture','no-update','update-allowed']:
            packets=list(original)
            for index in range(1,len(packets)):
                if mode!='toggle'and index!=1:continue
                nal=updates[index%2]if mode=='toggle'else updates[2 if mode=='active-texture'else int(mode in ['depth-only','no-update','update-allowed'])]
                packets[index]=struct.pack('>I',len(nal))+nal+packets[index]
            if mode in ['no-update','update-allowed']:
                # Active-parameter SEI: VPS0, one SPS0, self-contained=0.
                nal=bytes([78,1,129,1,7 if mode=='no-update'else 3,128])
                packets[0]=struct.pack('>I',len(nal))+nal+packets[0]
            def rewrite(tag,data):
                if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):data=b''.join(rewrite(t,p)for t,p in boxes(data))
                elif tag==b'stsd':data=data[:8]+b''.join(box(b'hev1'if t==b'hvc1'else t,p)for t,p in boxes(data[8:]))
                elif tag==b'stsz':data=data[:12]+b''.join(struct.pack('>I',len(p))for p in packets)
                elif tag==b'mdat':data=b''.join(packets)
                return box(tag,data)
            files[f'{name}-update-{mode}.mp4']=b''.join(rewrite(t,p)for t,p in roots)
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
