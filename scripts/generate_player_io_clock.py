#!/usr/bin/env python3
"""Authored valid MP4 for a controlled next-picture lookahead packet read stall;offline."""
from pathlib import Path
import hashlib,json
from generate_player_rate_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-io-clock'
def generate():
    files={n:b for n,b in source().items()if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Five authored16x16 VP9 keyframes,300ms each. The regression opens a valid source,decodes its first frame,and then gates actual reads of sample2 with an owned release guard. Packet data are unchanged;no private media,encoder,FFmpeg or network.','timescale':1000,'durations_ms':[300]*5,'gated_sample_index':2,'stop_ms':450,'sha256':{n:hashlib.sha256(b).hexdigest()for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
