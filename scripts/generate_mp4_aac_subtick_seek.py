"""Authored AAC960/VP9 with sub-media-tick finite edits; no encoder."""
from pathlib import Path
import struct,json,hashlib
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word,frame,mp4
from generate_mp4_audio_selection_regressions import make_audio_track
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-subtick-seek'
def generate():
    base=OUT.parent.parent/'audio';raw=(base/'aac-960-48000.m4a').read_bytes();movie=dict(atoms(dict(atoms(raw))[b'moov']));track=movie[b'trak'];tables=dict(atoms(dict(atoms(dict(atoms(track))[b'mdia']))[b'minf']))[b'stbl'];tables=dict(atoms(tables));entry=next(atoms(tables[b'stsd'][8:]))[1];assert int.from_bytes(tables[b'stsz'][8:12],'big')==7
    sizes=struct.unpack('>7I',tables[b'stsz'][12:]);offsets=struct.unpack('>'+str((len(tables[b'stco'])-8)//4)+'I',tables[b'stco'][8:]);sc=[struct.unpack('>III',tables[b'stsc'][i:i+12])for i in range(8,len(tables[b'stsc']),12)];packets=[];sample=0
    for chunk,offset in enumerate(offsets,1):
        n=next(e[1]for e in reversed(sc)if e[0]<=chunk)
        for _ in range(n):packets.append(raw[offset:offset+sizes[sample]]);offset+=sizes[sample];sample+=1
    assert sample==7
    video=dict(atoms(mp4([frame(8,'420',16,16)]*3,8,'420',size=(16,16),sync_samples=[1,2,3])));files={'reference.f32le':(base/'aac-960-48000-reference.f32le').read_bytes()};cases=[]
    for scale in [48000,50]:
        delta=960 if scale==48000 else 1
        for start,clock,length,version in [(0,1000,1,0),(1,1000,1,1),(0,4000,5,1)]:
            ftyp=atom(b'ftyp',video[b'ftyp']);audio=make_audio_track(2,3,b'mp4a',entry,packets,len(ftyp)+8+len(video[b'mdat']),scale,delta)
            parts=b''
            for k,b in atoms(audio[8:]):
                if k==b'tkhd':b=b[:20]+word(length)+b[24:]
                parts+=atom(k,b)
            media_start=start*delta;row=word(length,media_start,65536)if version==0 else struct.pack('>QqI',length,media_start,65536);audio=atom(b'trak',parts+atom(b'edts',atom(b'elst',word(version<<24,1)+row)))
            vm=dict(atoms(video[b'moov']));vtrack=b''
            for k,b in atoms(vm[b'trak']):
                if k==b'tkhd':b=b[:20]+word(3*clock//50)+b[24:]
                vtrack+=atom(k,b)
            mvhd=vm[b'mvhd'][:12]+word(clock,3*clock//50)+vm[b'mvhd'][20:96]+word(3)
            name=f'clock{scale}-start{start}-duration{length}-{clock}-v{version}.mp4';files[name]=ftyp+atom(b'mdat',video[b'mdat']+b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',vtrack)+audio);frames=length*48000//clock;assert frames*clock==length*48000;cases.append(dict(file=name,start_frames=start*960,frames=frames,rate=48000,scale=scale))
    files['manifest.json']=(json.dumps(dict(provenance='Owned arithmetic-coded AAC-LC960 payloads and stored reference PCM,plus freshly authored neutral VP9. Finite1/1.25ms V0/V1 edits on normal/coarse media grids;no private inputs,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
