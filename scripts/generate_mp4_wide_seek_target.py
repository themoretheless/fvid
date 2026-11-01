"""Regenerate own colored VP9 witnesses for wide seek request clamping."""
from pathlib import Path
import json,hashlib
from generate_mp4_seek_io_regressions import generate as colors
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-wide-seek-target'
def generate():
    files=colors();raw=files.pop('three-colors.mp4');cases=[]
    for scale in [50,90000]:
        step=scale//50
        for edited in [False,True]:
            def rewrite(data):
                out=b''
                for k,b in atoms(data):
                    if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=rewrite(b)
                    if k==b'trak' and edited:b+=atom(b'edts',atom(b'elst',word(0,1,2*step,0,65536)))
                    elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(scale,(2 if edited and k==b'mvhd' else 3)*step)+b[20:]
                    elif k==b'tkhd':b=b[:20]+word((2 if edited else 3)*step)+b[24:]
                    elif k==b'stts':b=word(0,1,3,step)
                    out+=atom(k,b)
                return out
            name=f'clock{scale}-'+('edit.mp4'if edited else 'full.mp4');files[name]=rewrite(raw);cases.append(dict(file=name,scale=scale,step=step,frames=2 if edited else 3))
    files['manifest.json']=(json.dumps(dict(provenance='Fresh owned arithmetic-coded colored VP9 keyframes and analytic YUV16 oracles. No private media,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
