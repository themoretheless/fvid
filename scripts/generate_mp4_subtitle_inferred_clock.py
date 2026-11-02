"""Authored VP9/tx3g controls for unspecified subtitle duration on signed clocks."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_mp4_subtitle_negative_clock import text_entry
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-subtitle-inferred-clock'
def generate_files():
    own=generate();parts=list(atoms(own['three-colors.mp4']));prefix=b''.join(atom(k,b) for k,b in parts[:2]);base=parts[2][1];track=next(b for k,b in atoms(base)if k==b'trak')
    texts=['first invented line','second invented line','last invented line'];packets=[struct.pack('>H',len(t))+t.encode()for t in texts]
    def subtitle(data,offset,scale,pts,identity=False):
        out=b''
        for k,b in atoms(data):
            if k in [b'mdia',b'minf']:b=subtitle(b,offset,scale,pts)
            elif k==b'tkhd':b=b[:12]+word(2)+b[16:20]+word(200)+b[24:]
            elif k==b'mdhd':b=b[:12]+word(scale,2)+b[20:]
            elif k==b'hdlr':b=word(0,0)+b'sbtl'+bytes(12)+b'Inferred clock\0'
            elif k==b'vmhd':k=b'nmhd';b=word(0)
            elif k==b'stbl':
                b=atom(b'stsd',word(0,1)+text_entry())+atom(b'stts',word(0,2,2,1,1,0))+atom(b'stsz',word(0,0,3,*map(len,packets)))+atom(b'stsc',word(0,1,1,3,1))+atom(b'stco',word(0,1,offset))+atom(b'ctts',word(1<<24,3)+b''.join(struct.pack('>Ii',1,p)for p in [pts[0],pts[1]-1,pts[2]-2]))
            out+=atom(k,b)
        if identity:out+=atom(b'edts',atom(b'elst',word(0,1,200,0,65536)))
        return out
    files={f'frame-{i}.yuv16':own[f'frame-{i}.yuv16']for i in range(3)};cases=[]
    for scale in [50,1001]:
        for mode,pts in [('negative-successor',[-2,1,-3]),('negative-last',[-3,-2,-1]),('wholly-negative',[-3*scale,-5*scale//2,-2*scale-1]),('positive',[0,1,3])]:
            for identity in [False,True]:
                name=f'{mode}-{scale}-{"identity"if identity else "plain"}.mp4';size=len(atom(b'moov',base+atom(b'trak',subtitle(track,0,scale,pts,identity))));sub=subtitle(track,len(prefix)+size+8,scale,pts,identity);files[name]=prefix+atom(b'moov',base+atom(b'trak',sub))+atom(b'mdat',b''.join(packets))
                cues=[]
                ordered=sorted(range(3),key=lambda i:pts[i])
                for position,i in enumerate(ordered):
                    p=pts[i]
                    successor=pts[ordered[position+1]]if position+1<len(ordered)else None
                    end=p+1 if i<2 else successor if successor is not None and successor>p else p+2*scale
                    if end<=0:continue
                    # Movie scale is50Hz; the identity edit covers4seconds.
                    end_ns=min(end*1_000_000_000//scale,4_000_000_000)if identity else end*1_000_000_000//scale
                    cues.append(dict(text=texts[i],start_ns=max(p,0)*1_000_000_000//scale,end_ns=end_ns))
                cases.append(dict(file=name,scale=scale,pts=pts,durations=[1,1,0],texts=texts,identity=identity,cues=cues))
    files['manifest.json']=(json.dumps(dict(provenance='Pure authored three-color16x16 VP9 plus invented visible tx3g text. Single terminal zero STTS duration and explicit signed CTTS; identity-edit and unedited controls. No private input, FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for name,data in generate_files().items():(OUT/name).write_bytes(data)
