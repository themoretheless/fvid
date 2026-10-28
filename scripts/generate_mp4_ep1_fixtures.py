"""Own three-AU EP1 movies with real separate ESC tracks and padb bit ends."""
import json,struct
from pathlib import Path
from generate_he_aac_packet_fixtures import box,boxes
from generate_aac_esds_stream_fixtures import descriptor
ROOT=Path(__file__).resolve().parents[1];SOURCE=ROOT/'tests/fixtures/playback-errors';OUT=SOURCE/'mp4-ep1'
def word(*values):return b''.join(struct.pack('>I',v)for v in values)
def make_movie(base,source,units,mode=None,allow_empty=False):
    original=dict(boxes(source));moov=list(boxes(original[b'moov']));tracks=[];audio=None
    for tag,body in moov:
        if tag!=b'trak':continue
        mdia=next(p for t,p in boxes(body)if t==b'mdia');hdlr=next(p for t,p in boxes(mdia)if t==b'hdlr')
        if hdlr[8:12]==b'soun':audio=body
        else:tracks.append(box(tag,body))
    assert audio is not None
    movie_scale=int.from_bytes(next(p for t,p in moov if t==b'mvhd')[12:16],'big')
    parts=list(boxes(source));assert parts[-1][0]==b'moov'
    prefix=b''.join(box(t,p)for t,p in parts[:-1]);media=bytearray();rows=base['frames'][:3]
    order=[u['stream_id']for u in rows[0]['units']];root=next(u['stream_id']for u in rows[0]['units']if u['depends_on']is None)
    asc=bytes.fromhex(base['asc']);skipped=False
    for ordinal,sid in enumerate(order):
        selected=[next(u for u in frame['units']if u['stream_id']==sid)for frame in rows]
        assert all(u['depends_on']==selected[0]['depends_on']for u in selected)
        omit=False
        if mode=='omit-empty' and not skipped and selected[0]['bits']==0:
            selected=selected[1:];skipped=True;omit=True
        if mode=='missing' and ordinal==0:selected=selected[:2]
        parent=selected[0]['depends_on']
        if mode=='fork' and ordinal==1:parent=root
        esid=root if mode=='duplicate'and ordinal==0 else sid
        head=struct.pack('>H',esid)+bytes([128 if parent is not None else 0])+(struct.pack('>H',parent)if parent is not None else b'')
        esds=bytes(4)+descriptor(3,head+descriptor(4,b'\x40\x14'+bytes(11)+descriptor(5,asc))+descriptor(6,b'\x02'))
        raw=[units[u['offset']:u['offset']+u['bytes']]for u in selected];assert allow_empty or all(raw)
        offset=len(prefix)+8+len(media);media.extend(b''.join(raw))
        padding=[(-u['bits'])%8 for u in selected];packed=bytes((padding[i]<<4)|(padding[i+1]if i+1<len(padding)else 7)for i in range(0,len(padding),2))
        scale=selected[0]['timescale'];step=selected[0]['duration']+(1 if mode=='duration'and ordinal==0 else 0);count=len(selected)
        tables={b'stts':word(0,1,count,step),b'stsz':word(0,0,count)+word(*(len(v)for v in raw)),b'stsc':word(0,1,1,count,1),b'stco':word(0,1,offset)}
        def rewrite(tag,body):
            if tag in [b'mdia',b'minf',b'stbl']:
                body=b''.join(rewrite(t,p)for t,p in boxes(body)if t not in [b'ctts',b'stss',b'padb'])
                if tag==b'stbl':
                    body+=box(b'padb',word(0,count)+packed)
                    if mode=='signed':body+=box(b'ctts',word(0x01000000,1,count,(-37*(scale//24000))&0xffffffff))
                    if omit:body+=box(b'ctts',word(0,1,count,step))
            elif tag==b'tkhd':body=body[:12]+word(ordinal+2)+body[16:20]+word(3 if mode=='edits'and sid==root else (3*base['n']*movie_scale+23999)//24000)+body[24:]
            elif tag==b'mdhd':body=body[:12]+word(scale,count*step)+body[20:]
            elif tag==b'stsd':body=body[:8]+b''.join(rewrite(t,p)for t,p in boxes(body[8:]))
            elif tag==b'mp4a':body=body[:28]+b''.join(rewrite(t,p)for t,p in boxes(body[28:]))
            elif tag==b'esds':body=esds
            elif tag in tables:body=tables[tag]
            return box(tag,body)
        track_body=b''.join(rewrite(t,p)for t,p in boxes(audio)if t!=b'edts')
        if mode=='edits' and sid==root:
            track_body+=box(b'edts',box(b'elst',word(0,3,1,0xffffffff,0x10000,1,7*scale//1000,0x10000,1,scale//1000,0x10000)))
        tracks.append(box(b'trak',track_body))
    nontracks=[]
    for tag,body in moov:
        if tag==b'trak':continue
        if tag==b'mvhd':body=body[:-4]+word(len(order)+2)
        nontracks.append(box(tag,body))
    return prefix+box(b'mdat',media)+box(b'moov',b''.join(nontracks+tracks))
def generate():
    parent=SOURCE/'aac-ep1';manifest=json.loads((parent/'manifest.json').read_bytes());units=(parent/'access-units.bin').read_bytes();files={};cases=[];bad=[];exports=[]
    names=['aac-er-lc-960-1-0-0','aac-ld-ltp-480-0','eld-n480-c1-x2-crc1','eld-n512-c7-x2-crc1']
    for base in manifest['cases']:
        if base['name']not in names:continue
        name=base['name'];source=(parent/base['reference_video']).read_bytes();files[name+'.mp4']=make_movie(base,source,units)
        frames=3*base['container_frame_samples'];files[name+'.f32le']=(parent/base['golden']).read_bytes()[:frames*base['channels']*4]
        cases.append(dict(name=name,video=name+'.mp4',golden=name+'.f32le',channels=base['channels'],rate=base['rate'],streams=len(base['frames'][0]['units']),frames=3))
        files[name+'-signed.mp4']=make_movie(base,source,units,'signed')
        cases.append(dict(name=name+'-signed',video=name+'-signed.mp4',golden=name+'.f32le',channels=base['channels'],rate=base['rate'],streams=len(base['frames'][0]['units']),frames=3))
        files[name+'-edits.mp4']=make_movie(base,source,units,'edits')
        pcm=(parent/base['golden']).read_bytes();stride=base['channels']*4;rate=base['rate']
        assert rate%30==0
        frames=rate//30
        files[name+'-edits.f32le']=bytes(frames*stride)+pcm[7*rate//1000*stride:(7*rate//1000+frames)*stride]+pcm[rate//1000*stride:(rate//1000+frames)*stride]
        exports.append(dict(name=name+'-edits',video=name+'-edits.mp4',golden=name+'-edits.f32le',channels=base['channels'],rate=rate,streams=len(base['frames'][0]['units']),frames=3))
        if name==names[0]:
            for mode,error in [('missing','EP1 MP4 missing simultaneous ESC access unit'),('duration','EP1 MP4 ESC source clocks disagree'),('fork','EP1 MP4 forked ES dependency'),('duplicate','EP1 MP4 duplicate ES ID')]:
                files[mode+'.mp4']=make_movie(base,source,units,mode);bad.append(dict(video=mode+'.mp4',mode=mode,error=error))
    assert len(cases)==8
    files['manifest.json']=(json.dumps(dict(cases=cases,exports=exports,malformed=bad,provenance='Own AVC video and real separate native EP1 ESC tracks; rational per-ES clocks, non-byte bit ends and deliberately nonzero excluded low bits. First three AUs and independent saved PCM. No private media, foreign codec, process or network.'),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
