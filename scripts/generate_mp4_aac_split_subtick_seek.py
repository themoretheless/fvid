"""Author split finite AAC960 edits from the owned sub-tick controls."""
from pathlib import Path
import json,hashlib
from generate_mp4_aac_subtick_seek import generate as controls
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-split-subtick-seek'
def generate():
    sources=controls();files={'reference.f32le':sources['reference.f32le']};cases=[]
    for scale,delta in [(48000,960),(50,1)]:
        data=sources[f'clock{scale}-start0-duration1-1000-v0.mp4'];parts=[]
        for k,b in atoms(data):
            if k==b'moov':
                tracks=[]
                for t,p in atoms(b):
                    if t==b'trak' and b'soun' in p:
                        inner=[]
                        for x,y in atoms(p):
                            if x==b'tkhd':y=y[:20]+word(2)+y[24:]
                            if x==b'edts':y=atom(b'elst',word(0,2)+word(1,0,65536)+word(1,delta,65536))
                            inner.append(atom(x,y))
                        p=b''.join(inner)
                    tracks.append(atom(t,p))
                b=b''.join(tracks)
            parts.append(atom(k,b))
        name=f'clock{scale}-two-1ms-edits.mp4';files[name]=b''.join(parts);cases.append(dict(file=name,frames=96,scale=scale))
    for scale in [48000,50]:
        source=files[f'clock{scale}-two-1ms-edits.mp4'];parts=[]
        for k,b in atoms(source):
            if k==b'moov':
                tracks=[]
                for t,p in atoms(b):
                    if t==b'trak' and b'soun' in p:
                        inner=[]
                        for x,y in atoms(p):
                            if x==b'tkhd':y=y[:20]+word(3)+y[24:]
                            if x==b'edts':
                                old=dict(atoms(y))[b'elst'];y=atom(b'elst',word(0,3)+word(1,0xffffffff,65536)+old[8:])
                            inner.append(atom(x,y))
                        p=b''.join(inner)
                    tracks.append(atom(t,p))
                b=b''.join(tracks)
            parts.append(atom(k,b))
        name=f'clock{scale}-silence-two-edits.mp4';files[name]=b''.join(parts);cases.append(dict(file=name,frames=144,scale=scale,leading_silence=48))
    files['manifest.json']=(json.dumps(dict(provenance='Owned AAC960 and neutral VP9; two finite 1ms edit spans starting at source frames0 and960. No private input,encoder or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
