#!/usr/bin/env python3
"""Authored300ms pictures for missed-refresh clock continuity;offline."""
from pathlib import Path
import hashlib,json
from generate_player_rate_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-late-clock'
def generate():
    files={n:b for n,b in source().items() if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Five authored16x16 VP9 colored keyframes,300ms each,1.5s total,and exact YUV oracles. A real800ms gap between presentation polls crosses the450ms stop bound while the old picture ends300ms and the next due picture starts600ms. No private media,FFmpeg,encoder or network.','timescale':1000,'picture_ms':300,'stall_ms':800,'stop_ms':450,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
