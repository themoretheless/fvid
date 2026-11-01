"""Owned AAC960/VP9 at49Hz, requiring recovery to the48kHz native lattice."""
from pathlib import Path
from fractions import Fraction as F
import json,hashlib,struct
from generate_mp4_aac_gap_subtick_seek import generate as gap
from generate_mp4_aac_split_subtick_seek import generate as split
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word,frame,mp4
from generate_mp4_audio_selection_regressions import make_audio_track,sowt_v0_header
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-recovered-silence-seek'
def generate():
    g=gap();s=split();files={'reference.f32le':g['reference.f32le']};cases=[]
    for name,source,leading in [('gap49.mp4',g['clock50-coded-then-gap.mp4'],False),('leading49.mp4',s['clock50-silence-two-edits.mp4'],True)]:
        def rewrite(data,audio=False):
            out=[]
            for k,b in atoms(data):
                if k==b'trak':b=rewrite(b,b'soun'in b)
                elif k in [b'moov',b'mdia',b'minf',b'stbl']:b=rewrite(b,audio)
                elif audio and k==b'mdhd':b=b[:12]+word(49)+b[16:]
                out.append(atom(k,b))
            return b''.join(out)
        files[name]=rewrite(source);begin=F(21,1000)-F(1,49);targets=[0,1,500001,1000000,1000001,1500001,1999999,2000000]if not leading else[0,1,500001,999999,1000000,1000001,2000000,2000001,2999999,3000000]
        windows=[(0,48,F(0)),(48,96,begin+F(20,48000))]if not leading else[(0,48,F(0)),(48,96,F(1,1000)),(96,144,F(2,1000)+F(20,1000)+F(20,48000)-F(1,49))]
        end=F(3 if leading else 2,1000);queries=[]
        for ns in targets+[18446744073709551615999999999]:
            wanted=F(ns,10**9);first=144 if leading else 96;at=end
            for a,b,origin in windows:
                skip=max(0,((wanted-origin)*48000).__ceil__())
                if skip<b-a:first=a+skip;at=origin+F(skip,48000);break
            if wanted>=end or at>=end:first=144 if leading else 96;at=end
            queries.append(dict(target_ns=str(ns),first=first,anchor_ns=at.numerator*10**9//at.denominator))
        cases.append(dict(file=name,leading=leading,frames=144 if leading else 96,duration_ns=int(end*10**9),queries=queries))
    # A later gap uses16kHz although rewind starts in an8kHz setup.
    video=dict(atoms(mp4([frame(8,'420',16,16)]*3,8,'420',size=(16,16),sync_samples=[1,2,3])));ftyp=atom(b'ftyp',video[b'ftyp']);offset=len(ftyp)+8+len(video[b'mdat']);packets=[struct.pack('<h',4096)*n for n in [64,63,63]]
    audio=make_audio_track(2,3,b'sowt',sowt_v0_header(1,8000),packets,offset,1000,8)
    def pcm_track(data):
        out=[]
        for k,b in atoms(data):
            if k in [b'mdia',b'minf',b'stbl']:b=pcm_track(b)
            elif k==b'tkhd':b=b[:20]+word(2)+b[24:]
            elif k==b'mdhd':b=b[:16]+word(16)+b[20:]
            elif k==b'stsd':b=word(0,2)+atom(b'sowt',sowt_v0_header(1,8000))+atom(b'sowt',sowt_v0_header(1,16000))
            elif k==b'stts':b=word(0,2,1,8,2,4)
            elif k==b'stsc':b=word(0,2,1,1,1,2,1,2)
            elif k==b'stco':b=word(0,3,offset,offset+128,offset+254)
            out.append(atom(k,b))
            if k==b'stsz':out.append(atom(b'ctts',word(0,3,1,0,1,12,1,18)))
        return b''.join(out)
    audio=atom(b'trak',pcm_track(audio[8:])+atom(b'edts',atom(b'elst',word(0,2,1,0,65536,1,24,65536))))
    movie=dict(atoms(video[b'moov']));mvhd=movie[b'mvhd'][:12]+word(1000,60)+movie[b'mvhd'][20:96]+word(3);vtrack=b''
    for k,b in atoms(movie[b'trak']):
        if k==b'tkhd':b=b[:20]+word(60)+b[24:]
        vtrack+=atom(k,b)
    files['changing-pcm-gap.mp4']=ftyp+atom(b'mdat',video[b'mdat']+b''.join(packets))+atom(b'moov',atom(b'mvhd',mvhd)+atom(b'trak',vtrack)+audio)
    queries=[];end=F(2,1000)
    for ns in [0,1,500001,999999,1000000,1000001,1100001,1500001,1999999,2000000,18446744073709551615999999999]:
        wanted=F(ns,10**9);first=24;at=end
        for a,b,origin,rate in [(0,8,F(0),8000),(8,24,F(1,1000),16000)]:
            skip=max(0,((wanted-origin)*rate).__ceil__())
            if skip<b-a:first=a+skip;at=origin+F(skip,rate);break
        if wanted>=end or at>=end:first=24;at=end
        queries.append(dict(target_ns=str(ns),first=first,anchor_ns=at.numerator*10**9//at.denominator))
    for c in cases:c.update(scale=49,first_duration=1)
    cases.append(dict(file='changing-pcm-gap.mp4',pcm=True,frames=24,scale=1000,first_duration=8,duration_ns=2000000,queries=queries))
    files['manifest.json']=(json.dumps(dict(provenance='Own AAC960 and neutral VP9 at49Hz. Exact rational native silence and recovered PCM phases;no private input,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
