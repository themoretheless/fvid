"""Authored positive-CTS VP9 movies; media duration and movie endpoint differ."""
import hashlib,json
from pathlib import Path
from generate_mp4_seek_io_regressions import generate
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-presentation-endpoint'
def main():
    controls=generate();base=controls['three-colors.mp4'];files={};cases=[]
    for duplicate in [False,True]:
      for shift in [0,2,10]:
        for unknown in [False,True]:
            end=shift+(4 if duplicate else 3)
            def rewrite(data):
                out=b''
                for k,b in atoms(data):
                    if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:
                        b=rewrite(b)
                        if k==b'stbl' and (shift or duplicate):
                            rows=word(0,3,1,shift,1,shift+1,1,shift) if duplicate else word(0,1,3,shift)
                            b+=atom(b'ctts',rows)
                    elif k==b'mvhd':b=b[:16]+word(end)+b[20:]
                    elif k==b'tkhd':b=b[:20]+word(end)+b[24:]
                    elif k==b'mdhd' and unknown:b=b[:16]+word(0xffffffff)+b[20:]
                    out+=atom(k,b)
                return out
            n=('duplicate-tail-' if duplicate else '')+f'shift-{shift}-'+('unknown' if unknown else 'known')+'.mp4';files[n]=rewrite(base)
            frames=[dict(sample=i,pts=shift+i,duration=1) for i in range(3)] if not duplicate else [dict(sample=0,pts=shift,duration=2),dict(sample=2,pts=shift+2,duration=2)]
            cases.append(dict(file=n,shift=shift,end=end,frames=frames,declared_media_duration=None if unknown else 3))
    files.update({n:b for n,b in controls.items() if n.endswith('.yuv16') or n.endswith('.packet')})
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps(dict(provenance='Pure authored three-color VP9 samples. Positive composition offsets 0/2/10 at 50 Hz; movie/track endpoints include the leading offset, media decode duration remains three ticks or unknown. No edit list, private media, encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest() for n,b in files.items()}),indent=2)+'\n')
if __name__=='__main__':main()
