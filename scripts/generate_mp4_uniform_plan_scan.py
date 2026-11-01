"""Own compact-index mono PCM with three neutral video pictures."""
from pathlib import Path
import json,hashlib,struct
from generate_vp9_chroma import atom,word,frame,mp4
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_audio_selection_regressions import sowt_v0_header
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-uniform-plan-scan'
def generate():
    video=dict(atoms(mp4([frame(8,'420',16,16)]*3,8,'420',size=(16,16),sync_samples=[1,2,3])));files={};cases=[]
    for count in [64,262144]:
        ftyp=atom(b'ftyp',video[b'ftyp']);offset=len(ftyp)+8+len(video[b'mdat'])
        stbl=atom(b'stsd',word(0,1)+atom(b'sowt',sowt_v0_header(1,48000)))+atom(b'stts',word(0,1,count,1))+atom(b'stsc',word(0,1,1,count,1))+atom(b'stsz',word(0,2,count))+atom(b'stco',word(0,1,offset))
        mdhd=bytearray(24);mdhd[12:20]=word(48000,count);audio=atom(b'mdia',atom(b'mdhd',mdhd)+atom(b'hdlr',word(0,0)+b'soun'+bytes(12))+atom(b'minf',atom(b'smhd',word(0,0))+atom(b'dinf',atom(b'dref',word(0,1)+atom(b'url ',word(1))))+atom(b'stbl',stbl)))
        tkhd=bytearray(84);tkhd[:4]=word(3);tkhd[12:16]=word(2);tkhd[20:24]=word((count*50+47999)//48000);tkhd[36:38]=struct.pack('>H',256);tkhd[40:44]=tkhd[56:60]=word(65536);tkhd[72:76]=word(1<<30)
        movie=dict(atoms(video[b'moov']));mvhd=movie[b'mvhd'][:16]+word(max(3,(count*50+47999)//48000))+movie[b'mvhd'][20:96]+word(3)
        name=f'frames-{count}.mp4';files[name]=ftyp+atom(b'mdat',video[b'mdat']+struct.pack('<h',4096)*count)+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',movie[b'trak'])+atom(b'trak',atom(b'tkhd',tkhd)+audio))
        cases.append(dict(file=name,frames=count,rate=48000,channels=1,pcm_value=0.125))
    # Small two-chunk controls distinguish selected header identity from entry ID.
    for mode in ['alias','unused','endian']:
        base=files['frames-64.mp4'];parts=dict(atoms(base));movie=dict(atoms(parts[b'moov']));tracks=list(atoms(parts[b'moov']));audio=next(b for k,b in tracks if k==b'trak' and b'soun' in b)
        chunk_start=len(atom(b'ftyp',parts[b'ftyp']))+8+len(video[b'mdat'])
        def change(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'mdia',b'minf',b'stbl']:b=change(b)
                elif k==b'stsd':b=word(0,2)+atom(b'sowt',sowt_v0_header(1,48000))+atom(b'twos'if mode!='alias'else b'sowt',sowt_v0_header(1,48000))
                elif k==b'stsc':b=word(0,1,1,64,1)if mode=='unused'else word(0,2,1,32,1,2,32,2)
                elif k==b'stco':b=word(0,1,chunk_start)if mode=='unused'else word(0,2,chunk_start,chunk_start+64)
                out+=atom(k,b)
            return out
        payload=parts[b'mdat']
        if mode=='endian':payload=payload[:-64]+struct.pack('>h',4096)*32
        moov=b''.join(atom(k,change(b)if k==b'trak'and b'soun'in b else b)for k,b in tracks)
        name=f'control-{mode}.mp4';files[name]=atom(b'ftyp',parts[b'ftyp'])+atom(b'mdat',payload)+atom(b'moov',moov);cases.append(dict(file=name,frames=64,rate=48000,channels=1,pcm_value=0.125,mode=mode))
    files['manifest.json']=(json.dumps(dict(provenance='Authored compact PCM sample tables,analytic constant waveform,and neutral VP9. Maximum audible duration5.462s;no private data,encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
