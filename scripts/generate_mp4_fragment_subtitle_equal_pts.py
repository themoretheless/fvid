"""Offline authored VP9/tx3g witnesses for missing ends at equal PTS."""
from pathlib import Path
import json,struct,hashlib
from generate_mp4_seek_io_regressions import generate as video_source
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_subtitle_negative_clock import text_entry
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-fragment-subtitle-equal-pts'
def generate():
    own=video_source();parts=list(atoms(own['three-colors.mp4']));moov=parts[2][1]
    track=next(b for k,b in atoms(moov)if k==b'trak')
    moov=b''.join(atom(k,b[:-4]+word(3)if k==b'mvhd'else b)for k,b in atoms(moov))
    texts=['first simultaneous','second simultaneous','later cue']
    packets=[struct.pack('>H',len(t))+t.encode()for t in texts]
    prefix=b''.join(atom(k,b)for k,b in parts[:2]);files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16']for i in range(3)};cases=[]
    for scale in [50,1001]:
        for name,pts,durations in [('equal',[0,0,1],[0,0,1]),('negative',[-1,-1,1],[0,0,1]),('terminal',[0,1,1],[1,0,0]),('explicit',[0,0,1],[1,1,1]),('distinct',[0,1,2],[1,0,0])]:
            dts=[0,durations[0],durations[0]+durations[1]]
            timing=[]
            for d in durations:
                if timing and timing[-1][1]==d:timing[-1][0]+=1
                else:timing.append([1,d])
            def subtitle(data,offset):
                out=b''
                for k,b in atoms(data):
                    if k in [b'mdia',b'minf']:b=subtitle(b,offset)
                    elif k==b'tkhd':b=b[:12]+word(2)+b[16:]
                    elif k==b'mdhd':b=b[:12]+word(scale,sum(durations))+b[20:]
                    elif k==b'hdlr':b=word(0,0)+b'sbtl'+bytes(12)+b'Own text\0'
                    elif k==b'vmhd':k=b'nmhd';b=word(0)
                    elif k==b'stbl':
                        b=atom(b'stsd',word(0,1)+text_entry())+atom(b'stts',word(0,0))+atom(b'stsz',word(0,0,0))+atom(b'stsc',word(0,0))+atom(b'stco',word(0,0))
                    out+=atom(k,b)
                return out
            def fragment(offset):
                rows=b''.join(word(d,len(packet))+struct.pack('>i',p-t)for d,packet,p,t in zip(durations,packets,pts,dts))
                return atom(b'moof',atom(b'mfhd',word(0,1))+atom(b'traf',atom(b'tfhd',word(0x20002,2,1))+atom(b'tfdt',word(0,0))+atom(b'trun',word((1<<24)|0xb01,3,offset)+rows)))
            name_moov=moov+atom(b'trak',subtitle(track,0))+atom(b'mvex',atom(b'trex',word(0,2,1,0,0,0)))
            filename=f'{name}-{scale}.mp4';files[filename]=prefix+atom(b'moov',name_moov)+fragment(len(fragment(0))+8)+atom(b'mdat',b''.join(packets))
            cues=[]
            for t,p,d in zip(texts,pts,durations):
                end=p+d if d else next((q for q in sorted(pts)if q>p),p+2*scale)
                if end>max(p,0):cues.append(dict(text=t,start_ns=max(p,0)*1_000_000_000//scale,end_ns=max(end,0)*1_000_000_000//scale))
            cues.sort(key=lambda c:c["start_ns"])
            cases.append(dict(file=filename,scale=scale,pts=pts,durations=durations,texts=texts,cues=cues))
    files['manifest.json']=(json.dumps(dict(provenance='Pure authored three-color16x16 VP9 with scalar YUV16 and three UTF-8 tx3g packets. Fragmented equal, signed-negative, distinct, explicit and terminal subtitle endpoints at50/1001Hz. No private media,encoder,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
