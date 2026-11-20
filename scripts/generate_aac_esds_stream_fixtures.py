"""Own bounded ES dependency/SL descriptors and AAC movies; offline only.
These qualify metadata retention, not EP1 track routing or SL packet decoding.
"""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import box,boxes
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'aac-esds-streams'
def descriptor(tag,data):
    n=len(data);digits=[n&127];n>>=7
    while n:digits.insert(0,(n&127)|128);n>>=7
    return bytes([tag])+bytes(digits)+data

def record(asc,es_id,flags,url,sl=b'\x02',duplicate=False):
    head=struct.pack('>H',es_id)+bytes([flags])
    if flags&128:head+=struct.pack('>H',17)
    if flags&64:head+=bytes([len(url)])+url
    if flags&32:head+=struct.pack('>H',65535)
    dc=descriptor(4,b'\x40\x14'+bytes(11)+descriptor(5,asc))
    tail=descriptor(0xe0,b'\x05\x02\xff\xff')+dc+(descriptor(6,sl)if sl is not None else b'')
    if duplicate:tail+=descriptor(6,b'\x02')
    return bytes(4)+descriptor(3,head+tail)

def movie(seed,esds):
    found=0
    def rewrite(tag,body):
        nonlocal found
        if tag==b'esds':found+=1;body=esds
        elif tag in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:body=b''.join(rewrite(t,p)for t,p in boxes(body))
        elif tag==b'stsd':body=body[:8]+b''.join(rewrite(t,p)for t,p in boxes(body[8:]))
        elif tag==b'mp4a':body=body[:28]+b''.join(rewrite(t,p)for t,p in boxes(body[28:]))
        return box(tag,body)
    # The template's terminal moov is the only growing region; media offsets
    # keep pointing to byte-identical mdat payloads before it.
    parts=list(boxes(seed));assert parts[-1][0]==b'moov'
    result=b''.join(rewrite(t,p)for t,p in parts);assert found==1;return result

def generate():
    parent=SOURCE/'aac-ep1';c=json.loads((parent/'manifest.json').read_bytes())['cases'][0];asc=bytes.fromhex(c['plain_asc']);seed=(parent/c['reference_video']).read_bytes();files={'reference.f32le':(parent/c['golden']).read_bytes()};cases=[];bad=[]
    for index,flags in enumerate([0,31,128,159,64,95,32,63,224,255]):
        url=b'\x05\x02\xff\0stream'if index%2 else bytes(range(255));sl=b'\x02'if index%3 else b'\0\xff\x05\x04\0';name=f'flags-{flags}';raw=record(asc,index*6553,flags,url,sl)
        files[name+'.esds']=raw;cases.append(dict(name=name,es_id=index*6553,depends_on=17 if flags&128 else None,ocr_es_id=65535 if flags&32 else None,url=url.hex()if flags&64 else None,priority=flags&31,sl_config=sl.hex(),asc=asc.hex()))
    raw=record(asc,0,0,b'',None);files['without-sl.esds']=raw;cases.append(dict(name='without-sl',es_id=0,depends_on=None,ocr_es_id=None,url=None,priority=0,sl_config=None,asc=asc.hex()))
    movies=[]
    for es_id,priority in [(0,0),(65535,31)]:
        name=f'root-{es_id}';raw=record(asc,es_id,priority,b'');files[name+'.esds']=raw;files[name+'.mp4']=movie(seed,raw);movies.append(dict(name=name,video=name+'.mp4',es_id=es_id,priority=priority))
    duplicate=record(asc,0,0,b'',duplicate=True);files['duplicate-sl.esds']=duplicate;files['duplicate-sl.mp4']=movie(seed,duplicate);bad.append(dict(name='duplicate-sl',video='duplicate-sl.mp4',error='duplicate SL configuration'))
    files['manifest.json']=(json.dumps(dict(cases=cases,movies=movies,malformed=bad,asc=asc.hex(),golden='reference.f32le',provenance='Own ES descriptors around owned synthetic sparse ER-LC audio and AVC video. Metadata-only dependent descriptors are not EP1 container-playback acceptance. No foreign codec, process, network or private media.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
