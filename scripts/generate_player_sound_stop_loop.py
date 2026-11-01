#!/usr/bin/env python3
"""Authored valid sources for sound-only stop versus A-B ordering;offline."""
from pathlib import Path
import json,hashlib
from generate_player_audio_failure_clock import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-sound-stop-loop'
def generate():
    files={n:b for n,b in source().items() if n not in ('manifest.json','sound.wav','sound.f32le')}
    files['manifest.json']=(json.dumps({'provenance':'Authored VP9 16x16 video control and 200ms mono 8kHz PCM. Actual audio worker uses a controlled output clock;no injected parsing faults,private media,FFmpeg or network.','audio_ms':200,'a_ms':25,'b_ms':100,'stop_ms':[75,100],'delayed_callback_ms':150,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
