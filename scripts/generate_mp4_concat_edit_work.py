"""Author short PCM MP4 edit-work witnesses; no encoders or private media."""
from pathlib import Path
import struct,json,hashlib
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_audio_selection_regressions import make_audio_track,sowt_v0_header
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-concat-edit-work'
def generate():
    files={}
    for name,count,frames,edited in [('early.m4a',3,160,True),('later.m4a',1,320,False)]:
        packets=[struct.pack('<h',1024*(i+1))*frames for i in range(count)]
        ftyp=atom(b'ftyp',b'isom'+word(0)+b'isomiso2')
        track=make_audio_track(1,3,b'sowt',sowt_v0_header(1,8000),packets,len(ftyp)+8,8000,frames)
        if edited:
            payload=track[8:]+atom(b'edts',atom(b'elst',word(0,1,frames,0,65536)))
            track=atom(b'trak',payload)
        mvhd=bytearray(100);mvhd[12:20]=word(8000,frames if edited else count*frames);mvhd[20:24]=word(65536);mvhd[24:26]=struct.pack('>H',256);mvhd[36:72]=word(65536,0,0,0,65536,0,0,0,0x40000000);mvhd[96:100]=word(2)
        files[name]=ftyp+atom(b'mdat',b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+track)
        files[name+'.f32le']=struct.pack('<f',1024/32768)*frames
    cases=[]
    base=OUT.parent/'mp4-open-ended-edit'
    meta=json.loads((base/'manifest-audio.json').read_text())['cases']
    for codec,rate in [('pcm',8000),('alac',8000),('aac',48000)]:
        c=next(c for c in meta if c['file']==f'{codec}-v0-clock{rate}-trim0-known.mp4')
        raw=(base/c['file']).read_bytes();channels=c['channels'];frames=c['frames']//3;step=c['media_ticks']//3
        oracle=(base/c['oracle']).read_bytes()
        for label,start,repeats,gap in [('early',0,1,False),('repeat',0,2,False),('preroll',1,1,False),('repeat-preroll',1,2,False),('gap',0,1,True)]:
            entries=([(frames,-1)] if gap else [])+[(frames,start*step)]*repeats
            duration=frames*(repeats+int(gap))
            def rewrite(data,audio=False):
                out=b''
                for k,b in atoms(data):
                    if k==b'trak' and b'vide' in b:continue
                    selected=audio or (k==b'trak' and b'soun' in b)
                    if k in [b'moov',b'trak',b'edts']:b=rewrite(b,selected)
                    elif k==b'mvhd':b=b[:16]+word(duration)+b[20:]
                    elif k==b'tkhd' and selected:b=b[:20]+word(duration)+b[24:]
                    elif k==b'elst' and selected:b=word(0,len(entries))+b''.join(struct.pack('>IiI',d,t,65536)for d,t in entries)
                    out+=atom(k,b)
                return out
            name=f'{codec}-{label}.m4a';files[name]=rewrite(raw)
            piece=oracle[start*frames*channels*4:(start+1)*frames*channels*4]
            files[name+'.f32le']=(bytes(frames*channels*4)if gap else b'')+piece*repeats
            work=(start+1)*repeats
            if codec=='aac' and start and repeats>1:work=start+repeats
            # Constant same-geometry PCM successor with one deliberately large packet.
            later=f'{codec}-later.m4a';count=2048//(2*channels)+1;packet=struct.pack('<h',1024)*(count*channels)
            ftyp=atom(b'ftyp',b'isom'+word(0)+b'isomiso2');track=make_audio_track(1,3,b'sowt',sowt_v0_header(channels,rate),[packet],len(ftyp)+8,rate,count)
            mvhd=bytearray(100);mvhd[12:20]=word(rate,count);mvhd[20:24]=word(65536);mvhd[24:26]=struct.pack('>H',256);mvhd[36:72]=word(65536,0,0,0,65536,0,0,0,0x40000000);mvhd[96:100]=word(2)
            files[later]=ftyp+atom(b'mdat',packet)+atom(b'moov',atom(b'mvhd',mvhd)+track);files[later+'.f32le']=struct.pack('<f',1024/32768)*(count*channels)
            cases.append(dict(file=name,oracle=name+'.f32le',later=later,work=work,frames=duration,channels=channels,rate=rate,later_frames=count,cap=2048))
    # Retain the authored hybrid's pictures and trim only its audio edit.
    raw=(base/'pcm-v0-clock8000-trim0-known.mp4').read_bytes()
    def video_control(data,audio=False):
        out=b''
        for k,b in atoms(data):
            selected=audio or(k==b'trak' and b'soun' in b)
            if k in [b'moov',b'trak',b'edts']:b=video_control(b,selected)
            elif k==b'elst' and selected:b=word(0,1,160,0,65536)
            elif k==b'tkhd' and selected:b=b[:20]+word(160)+b[24:]
            out+=atom(k,b)
        return out
    files['video-control.mp4']=video_control(raw)
    files['video-control.mp4.f32le']=(base/'pcm-oracle.f32le').read_bytes()[:160*4]
    files['manifest.json']=(json.dumps(dict(provenance='Authored constant signed PCM; first MP4 presents only one of three packets through a finite edit.',cases=cases,sha256={k:hashlib.sha256(v).hexdigest()for k,v in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
