"""Own three-color VP9 sample entries with exact and incomplete pasp fields."""
from pathlib import Path
import json,hashlib
from generate_mp4_seek_io_regressions import generate
from generate_vp9_chroma import atom,word
from generate_mp4_tfhd_defaults_regressions import atoms
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-pasp-sizes'
def main():
    owned=generate();base=owned['three-colors.mp4']
    def build(payload):
        def rewrite(data):
            out=b''
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
                elif k==b'stsd':b=b[:8]+rewrite(b[8:])
                elif k==b'vp09' and payload is not None:b+=atom(b'pasp',payload)
                out+=atom(k,b)
            return out
        return rewrite(base)
    files={'absent.mp4':base,'wide.mp4':build(word(4,2)),'zero-fallback.mp4':build(word(0,1))}
    bad=[]
    for size in [0,4,7,9,12]:
        n=f'pasp-{size}-bytes.mp4';files[n]=build((word(4,2)+word(0))[:size]);bad.append(n)
    for i in range(3):files[f'frame-{i}.yuv16']=owned[f'frame-{i}.yuv16']
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in files.items():(OUT/n).write_bytes(b)
    (OUT/'manifest.json').write_text(json.dumps({'provenance':'Own 3-frame 50Hz VP9 colored keyframes and analytic YUV16. Only sample-entry pasp differs; media bytes/offsets remain identical. No private media/parameters, FFmpeg or network.','invalid':bad,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n')
if __name__=='__main__':main()
