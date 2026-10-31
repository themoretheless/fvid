#!/usr/bin/env python3
"""Authored long pictures expose actual three-second relative navigation;offline."""
from pathlib import Path
import hashlib,json
from generate_mp4_frame_step import generate as source
from generate_mp4_fragment_description_regressions import atoms
from generate_vp9_chroma import atom,word
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-jump-clock'
def generate():
    seed=source();files={f'frame-{i}.yuv16':seed[f'frame-{i}.yuv16'] for i in range(3)}
    def walk(body):
        out=b''
        for k,b in atoms(body):
            if k in [b'moov',b'trak',b'mdia',b'minf',b'stbl']:b=walk(b)
            elif k in [b'mvhd',b'mdhd']:b=b[:12]+word(1000,10000)+b[20:]
            elif k==b'tkhd':b=b[:20]+word(10000)+b[24:]
            elif k==b'stts':b=word(0,1,5,2000)
            out+=atom(k,b)
        return out
    files['movie.mp4']=walk(seed['cfr.mp4'])
    files['manifest.json']=(json.dumps({'provenance':'Five authored16x16 VP9 colored keyframes,2s each,10s total. Independent YUV planes. Tiny packet data;long picture intervals expose actual3s keyboard jump origin. No private media,FFmpeg,encoder or network.','timescale':1000,'durations':[2000]*5,'jump_ms':3000,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
