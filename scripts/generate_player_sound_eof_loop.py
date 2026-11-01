#!/usr/bin/env python3
"""Authored valid sources for native sound EOF A-B ownership;offline."""
from pathlib import Path
import json,hashlib
from generate_player_sound_stop_loop import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-sound-eof-loop'
def generate():
    files={n:b for n,b in source().items() if n!='manifest.json'}
    files['manifest.json']=(json.dumps({'provenance':'Authored five16x16 VP9 keyframes and200ms mono8kHz scalar PCM with exact YUV/PCM oracles. A real audio worker drains its output;Pause/Play command acknowledgments fence actual EOF delivery without consuming its event. No private media,FFmpeg,encoder or network.','audio_ms':200,'a_ms':25,'b_ms':[100,200,300],'stop_ms':75,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
