"""Own depth-scalability VPS, SPS and DLT PPS metadata; no picture qualification."""
import json,struct
from pathlib import Path
import generate_hevc_layered_sps as g
import generate_hevc_layered_pps as p
import generate_hevc_depth_tables as t
from generate_he_aac_packet_fixtures import boxes
from generate_hevc_extension_tail_fixtures import extended
from hevc_fixture_mp4 import box
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/hevc-depth-pps-binding'
def generate():
    seed=(g.D.parent/'hevc/main-ipb.mp4').read_bytes()
    def child(data,tag):return next(p for t,p in boxes(data)if t==tag)
    stbl=child(child(child(child(dict(boxes(seed))[b'moov'],b'trak'),b'mdia'),b'minf'),b'stbl')
    entry=next(boxes(child(stbl,b'stsd')[8:]))[1]
    width,height=struct.unpack_from('>HH',entry,24)
    original=child(entry[78:],b'hvcC');at=23;units={}
    for _ in range(original[22]):
        kind=original[at]&63;count=int.from_bytes(original[at+1:at+3],'big');at+=3
        units[kind]=[]
        for _ in range(count):
            n=int.from_bytes(original[at:at+2],'big');at+=2;units[kind].append(original[at:at+n]);at+=n
    target=g.fmt(chroma=0,depth=(8,8),size=(width,height));target['crop']=[0]*4
    base=g.fmt(chroma=1,depth=(8,8),size=(width,height));base['crop']=[0]*4
    vps,_,_=g.vps(target,depth_layer=True,base_format=base)
    files={'depth.vps':vps,'texture.sps':units[33][0],'depth.sps':g.sps(multilayer=True,sps_id=1)}
    source=t.generate();manifest=json.loads(source['manifest.json']);cases=[]
    for name in ['variable','flags-8','empty','duplicates','delta-8','bad-depth','predicted-xor']:
        c=next(c for c in manifest['cases']if c['name']==name)
        bits=''.join(f'{b:08b}'for b in source[name+'.bin'])[:c['bits']]
        if name=='bad-depth':bits=t.header(10,1)+'0'
        error='HEVC DLT depth disagrees with SPS'if name=='bad-depth'else 'HEVC DLT layer count disagrees with VPS'if name=='predicted-xor'else None
        for future in [False,True]:
            label=name+('-future'if future else '')
            files[label+'.pps']=p.pps(layer=3,id=1,future=future,depth_tables=bits,sps_id=1)
            files[label+'-texture.pps']=p.pps(layer=0,id=1,future=future,depth_tables=bits)
            cases.append(dict(name=label,expected=None if error else c['expected'][0],error=error))
    # Complete short base-texture pictures with DLT metadata for an unused depth
    # layer. This is base-layer playback acceptance, not depth-picture support.
    payload=t.header(8,1)+'100'+t.delta(8,[1,2,4,9])
    texture_pps=extended(units[34][0],0,payload,extension_flags='00100000')
    def rewrite(tag,data):
        if tag in (b'moov',b'trak',b'mdia',b'minf',b'stbl'):
            data=b''.join(rewrite(k,p)for k,p in boxes(data))
        elif tag==b'stsd':
            hvcc=bytearray(original[:23]);hvcc[12]=120;hvcc[22]=3
            for kind,nals in [(32,[vps]),(33,[units[33][0],files['depth.sps']]),(34,[texture_pps,p.pps(layer=3,id=1,depth_tables=payload,sps_id=1)])]:
                hvcc+=bytes([0x80|kind])+struct.pack('>H',len(nals))
                for nal in nals:hvcc+=struct.pack('>H',len(nal))+nal
            data=data[:8]+b''.join(box(k,p[:78]+b''.join(box(t,bytes(hvcc)if t==b'hvcC'else v)for t,v in boxes(p[78:])))for k,p in boxes(data[8:]))
        return box(tag,data)
    files['texture-playback.mp4']=b''.join(rewrite(k,p)for k,p in boxes(seed))
    files['manifest.json']=(json.dumps(dict(provenance='Own complete metadata NALs. Context/binding acceptance only; not a full conformant active-3D picture stream.',cases=cases),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
