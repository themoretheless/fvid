"""Authored inactive PPS3D extension and active/truncated controls, offline."""
import struct
from generate_he_aac_packet_fixtures import DEST,boxes
from hevc_fixture_mp4 import box
from generate_hevc_extension_tail_fixtures import extended

def generate():
    seed=(DEST.parent/'hevc/main-ipb.mp4').read_bytes();roots=list(boxes(seed));assert roots[-1][0]==b'moov'
    variants=[('inactive','00100000','0'),('inactive-future','00100001','0'+'10100101'),('active','00100000','1'+'000000'+'0000'+'1'+'0'+'0'+'00000000'),('missing-flag','00100000','')]
    for label,flags,tail in variants:
        def config(data):
            result=bytearray(data[:23]);at=23
            for _ in range(data[22]):
                header=data[at:at+3];at+=3;result+=header
                for _ in range(int.from_bytes(header[1:],'big')):
                    n=int.from_bytes(data[at:at+2],'big');at+=2;nal=data[at:at+n];at+=n
                    if header[0]&63==34:nal=extended(nal,0,tail,extension_flags=flags)
                    result+=struct.pack('>H',len(nal))+nal
            assert at==len(data);return bytes(result)
        def rewrite(tag,data):
            if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):
                data=b''.join(rewrite(t,p)for t,p in boxes(data))
            elif tag==b'stsd':
                data=data[:8]+b''.join(box(t,p[:78]+b''.join(box(k,config(v)if k==b'hvcC'else v)for k,v in boxes(p[78:])))for t,p in boxes(data[8:]))
            return box(tag,data)
        (DEST/f'hevc-3d-pps-{label}-synthetic.mp4').write_bytes(b''.join(rewrite(t,p)for t,p in roots))
    def child(data,tag):return next(p for t,p in boxes(data)if t==tag)
    stbl=child(child(child(child(dict(roots)[b'moov'],b'trak'),b'mdia'),b'minf'),b'stbl')
    sizes=child(stbl,b'stsz');constant,count=struct.unpack_from('>II',sizes,4);assert constant==0
    lengths=struct.unpack('>'+str(count)+'I',sizes[12:])
    chunks=child(stbl,b'stco');assert int.from_bytes(chunks[4:8],'big')==1
    at=int.from_bytes(chunks[8:12],'big');original=[]
    for n in lengths:original.append(seed[at:at+n]);at+=n
    for label,_,_ in variants:
        data=(DEST/f'hevc-3d-pps-{label}-synthetic.mp4').read_bytes()
        tree=dict(boxes(data))
        table=child(child(child(child(tree[b'moov'],b'trak'),b'mdia'),b'minf'),b'stbl')
        entry=next(boxes(child(table,b'stsd')[8:]))[1];hvcc=child(entry[78:],b'hvcC')
        at=23;update=None
        for _ in range(hvcc[22]):
            kind=hvcc[at]&63;count=int.from_bytes(hvcc[at+1:at+3],'big');at+=3
            for _ in range(count):
                n=int.from_bytes(hvcc[at:at+2],'big');at+=2;nal=hvcc[at:at+n];at+=n
                if kind==34:update=nal
        assert update is not None
        packets=list(original);packets[1]=struct.pack('>I',len(update))+update+packets[1]
        def rewrite_update(tag,data):
            if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):
                data=b''.join(rewrite_update(t,p)for t,p in boxes(data))
            elif tag==b'stsd':
                data=data[:8]+b''.join(box(b'hev1'if t==b'hvc1'else t,p)for t,p in boxes(data[8:]))
            elif tag==b'stsz':data=data[:12]+b''.join(struct.pack('>I',len(p))for p in packets)
            elif tag==b'mdat':data=b''.join(packets)
            return box(tag,data)
        (DEST/f'hevc-3d-pps-{label}-inband-synthetic.mp4').write_bytes(b''.join(rewrite_update(t,p)for t,p in roots))
if __name__=='__main__':generate()
