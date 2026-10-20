"""Authored MP4 enabled/movie selection; preserve legacy all-zero flag controls."""
import json,hashlib,struct
from pathlib import Path
from generate_vp9_chroma import frame,mp4
from generate_audio_resample_window_fixture import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-enabled-tracks'
def boxes(data):
    at=0
    while at<len(data):
        size=int.from_bytes(data[at:at+4],'big');assert size>=8;yield data[at+4:at+8],data[at+8:at+size];at+=size
    assert at==len(data)
def canonical_movie(body,next_id):
    body=bytearray(body);body[20:24]=word(65536);body[24:28]=word(0x01000000)
    for at,value in [(36,65536),(52,65536),(68,1<<30),(96,next_id)]:body[at:at+4]=word(value)
    return body
def rewrite_track(data,number,flags,offset,version):
    output=b''
    for kind,body in boxes(data):
        if kind in [b'mdia',b'minf',b'stbl']:body=rewrite_track(body,number,flags,offset,version)
        elif kind==b'tkhd':
            body=bytearray(body);body[:4]=word(flags);body[12:16]=word(number)
            if version:body=bytes([1])+body[1:4]+bytes(16)+body[12:20]+struct.pack('>Q',int.from_bytes(body[20:24],'big'))+body[24:]
        elif kind==b'stco':body=word(0,1,offset)
        output+=atom(kind,body)
    return output
def clip(first,second,version=False,faststart=False):
    sources=[dict(boxes(mp4([frame(8,'420',side,side)]*3,8,'420',(side,side),sync_samples=[1,2,3]))) for side in [16,64]]
    ftyp=atom(b'ftyp',sources[0][b'ftyp']);payloads=[s[b'mdat'] for s in sources];mdat=atom(b'mdat',b''.join(payloads))
    movie=canonical_movie(dict(boxes(sources[0][b'moov']))[b'mvhd'],3);tracks=[dict(boxes(s[b'moov']))[b'trak'] for s in sources]
    offset=len(ftyp)+8
    def moov(offset):return atom(b'moov',atom(b'mvhd',movie)+b''.join(atom(b'trak',rewrite_track(track,index+1,[first,second][index],offset+sum(map(len,payloads[:index])),version)) for index,track in enumerate(tracks)))
    metadata=moov(offset)
    if faststart:metadata=moov(offset+len(metadata));return ftyp+metadata+mdat
    return ftyp+mdat+metadata

def sample_track(number,flags,handler,codec,entry,packets,offset,rate,delta):
    stbl=atom(b'stsd',word(0,1)+atom(codec,entry))+atom(b'stts',word(0,1,len(packets),delta))+atom(b'stsc',word(0,1,1,len(packets),1))+atom(b'stsz',word(0,0,len(packets),*map(len,packets)))+atom(b'stco',word(0,1,offset))
    dinf=atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))
    mdhd=bytearray(24);mdhd[12:20]=word(rate,len(packets)*delta)
    mdia=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+handler+bytes(12))+atom(b'minf',dinf+atom(b'stbl',stbl)))
    tkhd=bytearray(84);tkhd[:4]=word(flags);tkhd[12:16]=word(number);tkhd[20:24]=word(3);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(0x40000000)
    return atom(b'trak',atom(b'tkhd',tkhd)+mdia+atom(b'udta',atom(b'name',b'enabled' if flags==3 else b'disabled')))
def audiovisual(audio_widths=(16,16),audio_flags=(6,3)):
    base=dict(boxes(mp4([frame(8,'420',16,16)]*3,8,'420',(16,16),sync_samples=[1,2,3])));ftyp=atom(b'ftyp',base[b'ftyp']);payloads=[base[b'mdat']];movie=dict(boxes(base[b'moov']));tracks=atom(b'trak',rewrite_track(movie[b'trak'],1,3,len(ftyp)+8,False));expected=bytearray()
    desired=[]
    for index in range(3):
        samples=[((index*160+i)*37)%32768-16384 for i in range(160)]
        desired.append(b''.join(struct.pack('<h',v) for v in samples))
        for value in samples:expected+=struct.pack('<f',value/32768.)
    for number,handler,codec,packets in [(2,b'soun',b'sowt',[bytes(320)]*3),(3,b'soun',b'sowt',desired),(4,b'sbtl',b'tx3g',[b'\0\x0ddisabled text']*3),(5,b'sbtl',b'tx3g',[len(f'enabled text{i}').to_bytes(2,'big')+f'enabled text{i}'.encode() for i in range(3)])]:
        entry=bytearray(28 if handler==b'soun' else 38);entry[6:8]=b'\0\1'
        if handler==b'soun':entry[16:20]=struct.pack('>HH',1,audio_widths[number-2]);entry[24:28]=word(8000<<16)
        tracks+=sample_track(number,audio_flags[number-2] if handler==b'soun' else (6 if number%2==0 else 3),handler,codec,entry,packets,len(ftyp)+8+sum(map(len,payloads)),8000 if handler==b'soun' else 50,160 if handler==b'soun' else 1);payloads.append(b''.join(packets))
    return ftyp+atom(b'mdat',b''.join(payloads))+atom(b'moov',atom(b'mvhd',canonical_movie(movie[b'mvhd'],6))+tracks),bytes(expected)
def generate():
    files={};cases=[]
    for name,a,b,v,fast,side in [
        ('legacy-zero',0,0,False,False,16),('both-enabled',3,3,False,False,16),('disabled-first',6,3,False,False,64),
        ('zero-first',0,3,False,False,64),('preview-first',5,3,False,False,64),('enabled-nonmovie-first',1,3,False,False,64),
        ('preview-only-choice',6,5,False,False,64),('enabled-only-choice',0,1,False,False,64),
        ('version1-disabled',6,3,True,False,64),('faststart-disabled',6,3,False,True,64)]:
        filename=name+'.mp4';files[filename]=clip(a,b,v,fast);cases.append({'file':filename,'side':side,'flags':[a,b]})
    files['audiovisual.mp4'],files['expected-audio.f32le']=audiovisual()
    files['manifest.json']=(json.dumps({'provenance':'Ten authored60ms neutral16/64-square VP9 MP4 pairs with tkhd flags, v0/v1 and faststart layouts. No private media/encoder/network.','spec':'https://developer.apple.com/documentation/quicktime-file-format/Track_header_atom/Flags','cases':cases,'sha256':{k:hashlib.sha256(v).hexdigest() for k,v in files.items()}},indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
