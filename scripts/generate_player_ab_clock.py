#!/usr/bin/env python3
"""Authored valid A-B ranges and within-picture marker clocks;offline."""
from pathlib import Path
import hashlib,json
from generate_player_rate_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-ab-clock'
def generate():
    files={n:b for n,b in source().items() if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Five authored16x16 VP9 pictures,300ms each,1.5s total,and independent YUV oracles. Valid A-B ranges300..600ms and1200..1500ms;markers75ms and175ms lie inside the first picture. No private media,encoder,FFmpeg or network.','timescale':1000,'durations':[300]*5,'ranges_ms':[[75,175],[300,600],[1200,1500]],'marker_phases_ms':[75,175],'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
