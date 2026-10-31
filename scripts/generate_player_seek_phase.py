#!/usr/bin/env python3
"""Authored covering-picture witness for sub-frame seek phase;offline."""
from pathlib import Path
import hashlib,json
from generate_player_jump_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-seek-phase'
def generate():
    files={n:b for n,b in source().items() if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Only authored five16x16 VP9 colored pictures,2s each,10s total,and independent YUV oracles. Requested3300ms lies inside picture2000..4000ms. No private media,FFmpeg,encoder or network.','timescale':1000,'seek_ms':3300,'covering_picture_ms':[2000,4000],'remaining_ms':700,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
