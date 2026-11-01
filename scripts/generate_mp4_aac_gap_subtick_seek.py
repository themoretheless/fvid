"""Authored AAC960 source gap with two finite video-backed edit spans."""
from pathlib import Path
import json,hashlib
from generate_mp4_aac_split_subtick_seek import generate as split
from generate_mp4_tfhd_defaults_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/mp4-aac-gap-subtick-seek'
def generate():
    original=split();files={'reference.f32le':original['reference.f32le']};cases=[]
    for scale,delta in [(48000,960),(50,1)]:
        def rewrite(data):
            result=[]
            for k,b in atoms(data):
                if k in [b'moov',b'trak',b'mdia',b'minf']:b=rewrite(b)
                elif k==b'stbl' and b'mp4a' in b:b+=atom(b'ctts',word(0,2,1,0,6,delta))
                result.append(atom(k,b))
            return b''.join(result)
        name=f'clock{scale}-coded-then-gap.mp4';files[name]=rewrite(original[f'clock{scale}-two-1ms-edits.mp4']);cases.append(dict(file=name,frames=96,scale=scale))
    files['manifest.json']=(json.dumps(dict(provenance='Owned AAC960 payloads and neutral VP9. Packet0 spans0..20ms; remaining packets start40ms. Two finite edits expose1ms of source audio then1ms of declared source gap. No private inputs,FFmpeg or network.',cases=cases,sha256={n:hashlib.sha256(b).hexdigest()for n,b in files.items()}),indent=2)+'\n').encode();return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
