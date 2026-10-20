"""Own opaque parameter tails and no-update guidance over synthetic pictures."""
import struct,json
from pathlib import Path
from generate_he_aac_packet_fixtures import DEST,boxes
from generate_hevc_extension_tail_fixtures import extended
from hevc_fixture_mp4 import box
OUT=DEST/'hevc-raw-parameter-identity'
def generate():
    files={};cases=[]
    def child(d,t):return next(p for k,p in boxes(d)if k==t)
    for name in ['main-ipb','main10-ipb']:
        seed=(DEST.parent/'hevc'/f'{name}.mp4').read_bytes();roots=list(boxes(seed));assert roots[-1][0]==b'moov'
        stbl=child(child(child(child(dict(roots)[b'moov'],b'trak'),b'mdia'),b'minf'),b'stbl')
        sizes=child(stbl,b'stsz');constant,count=struct.unpack_from('>II',sizes,4);assert constant==0
        lengths=struct.unpack('>'+str(count)+'I',sizes[12:]);chunks=child(stbl,b'stco');assert int.from_bytes(chunks[4:8],'big')==1
        at=int.from_bytes(chunks[8:12],'big');original=[]
        for n in lengths:original.append(seed[at:at+n]);at+=n
        entry=next(boxes(child(stbl,b'stsd')[8:]))[1];hvcc=child(entry[78:],b'hvcC');at=23;units={}
        for _ in range(hvcc[22]):
            kind=hvcc[at]&63;count=int.from_bytes(hvcc[at+1:at+3],'big');at+=3
            for _ in range(count):
                n=int.from_bytes(hvcc[at:at+2],'big');at+=2;units[kind]=hvcc[at:at+n];at+=n
        for kind,label in [(32,'vps'),(33,'sps'),(34,'pps')]:
            changed=extended(units[kind],1,'10100101')
            files[f'{name}-{label}-original.nal']=units[kind];files[f'{name}-{label}-changed.nal']=changed
            for mode in ['forbidden','allowed','identical','change-return','late-claim','same-packet-claim','revoked','late-clean-claim','late-return-claim','same-packet-revoked']:
                packets=list(original);sei=bytes([78,1,129,1,3 if mode in ['allowed','late-claim','same-packet-claim','late-clean-claim','late-return-claim']else 7,128]);packets[0]=struct.pack('>I',len(sei))+sei+packets[0]
                if mode=='same-packet-revoked':
                    cleared=bytes([78,1,129,1,3,128]);packets[0]=struct.pack('>I',len(sei))+sei+struct.pack('>I',len(cleared))+cleared+original[0]
                updates=[units[kind]]if mode=='identical'else[changed,units[kind]]if mode in ['change-return','late-return-claim']else[changed]
                prefix=b''.join(struct.pack('>I',len(p))+p for p in updates);packets[1]=prefix+packets[1]
                if mode in ['late-claim','same-packet-claim','late-clean-claim','revoked','late-return-claim']:
                    declaration=bytes([78,1,129,1,3 if mode=='revoked'else 7,128])
                    prefix_sei=struct.pack('>I',len(declaration))+declaration
                    if mode in ['late-claim','late-return-claim']:packets[2]=prefix_sei+original[2]
                    elif mode=='same-packet-claim':packets[1]=prefix+prefix_sei+original[1]
                    elif mode=='revoked':
                        packets[1]=prefix_sei+original[1]
                        packets[2]=struct.pack('>I',len(changed))+changed+original[2]
                    else:
                        same=struct.pack('>I',len(units[kind]))+units[kind]
                        packets[1]=same+prefix_sei+original[1];packets[2]=same+original[2]
                def rewrite(tag,data):
                    if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):data=b''.join(rewrite(t,p)for t,p in boxes(data))
                    elif tag==b'stsd':data=data[:8]+b''.join(box(b'hev1'if t==b'hvc1'else t,p)for t,p in boxes(data[8:]))
                    elif tag==b'stsz':data=data[:12]+b''.join(struct.pack('>I',len(p))for p in packets)
                    elif tag==b'mdat':data=b''.join(packets)
                    return box(tag,data)
                file=f'{name}-{label}-{mode}.mp4';files[file]=b''.join(rewrite(t,p)for t,p in roots)
                cases.append(dict(file=file,seed=name,kind=label,mode=mode,refusal=mode not in ['allowed','identical','late-clean-claim']))
        large=extended(units[33],1,'10'*16000)
        assert len(large)+4<4096
        files[f'{name}-sps-large.nal']=large
        def large_config(data):
            out=bytearray(data[:23]);at=23
            for _ in range(data[22]):
                header=data[at:at+3];at+=3;out+=header
                for _ in range(int.from_bytes(header[1:],'big')):
                    n=int.from_bytes(data[at:at+2],'big');at+=2;nal=data[at:at+n];at+=n
                    if header[0]&63==33:nal=large
                    out+=struct.pack('>H',len(nal))+nal
            assert at==len(data);return bytes(out)
        def large_movie(tag,data):
            if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):data=b''.join(large_movie(t,p)for t,p in boxes(data))
            elif tag==b'stsd':data=data[:8]+b''.join(box(t,p[:78]+b''.join(box(k,large_config(v)if k==b'hvcC'else v)for k,v in boxes(p[78:])))for t,p in boxes(data[8:]))
            return box(tag,data)
        file=f'{name}-sps-large-config.mp4';files[file]=b''.join(large_movie(t,p)for t,p in roots)
        cases.append(dict(file=file,seed=name,kind='sps',mode='large-config',refusal=False))
    files['manifest.json']=(json.dumps(dict(provenance='Own opaque extension bits and active-parameter SEI; unchanged synthetic I/P/B payloads. Exact raw identity, not active tool support.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
