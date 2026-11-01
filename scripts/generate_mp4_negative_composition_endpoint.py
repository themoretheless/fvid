"""Authored VP9 with signed composition preroll and independent movie endpoint."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_seek_io_regressions import generate as seed
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-negative-composition-endpoint'
def generate():
    base=seed();files={n:b for n,b in base.items() if n.endswith('.yuv16')};cases=[]
    for shift,mode in [(s,m)for s in [-2,-1,0,1]for m in ["known","unknown"]]+[(-1,"declared-tail")]:
        media_duration=5 if mode=="declared-tail" else 3
        end=5 if mode=="declared-tail" else 3+shift
        header=0xffffffff if mode=="unknown" else end
        def rewrite(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                    b=rewrite(b)
                    if k==b'stbl':b+=atom(b'ctts',word(1<<24,1,3)+struct.pack('>i',shift))
                elif k==b'mvhd':b=b[:16]+word(header)+b[20:]
                elif k==b'tkhd':b=b[:20]+word(header)+b[24:]
                elif k==b'mdhd':b=b[:16]+word(media_duration)+b[20:]
                out+=atom(k,b)
            return out
        name=f'shift-{shift}-{mode}.mp4';files[name]=rewrite(base['three-colors.mp4']);cases.append(dict(file=name,shift=shift,end=end,media_duration=media_duration,header=header))
    files['manifest.json']=(json.dumps(dict(provenance='Three authored independent colored VP9 keyframes. Version1 ctts signed uniform composition offset; media decode duration3ticks,finite movie/track composition endpoint3+offset at50Hz. Negative samples are preroll. No private media,encoder,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
