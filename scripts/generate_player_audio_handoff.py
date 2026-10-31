#!/usr/bin/env python3
"""Authored video with a shorter audio clock, no encoder or network."""
from pathlib import Path
import hashlib,json,struct
from generate_mp4_frame_step import generate as source
OUT=Path(__file__).resolve().parents[1]/'tests/fixtures/playback-errors/player-audio-handoff'
def generate():
    seed=source();files={'movie.mp4':seed['cfr.mp4']}
    files.update({f'frame-{i}.yuv16':seed[f'frame-{i}.yuv16'] for i in range(3)})
    samples=[(i*17%1024)-512 for i in range(320)]
    raw=struct.pack('<320h',*samples)
    files['sound.wav']=b'RIFF'+struct.pack('<I',36+len(raw))+b'WAVEfmt '+struct.pack('<IHHIIHH',16,1,1,8000,16000,2,16)+b'data'+struct.pack('<I',len(raw))+raw
    files['sound.f32le']=b''.join(struct.pack('<f',v/32768) for v in samples)
    files['manifest.json']=(json.dumps({'provenance':'Authored five16x16 VP9 keyframes at50Hz and40ms mono8kHz scalar PCM. No private media,encoder or network.','video_ms':100,'audio_ms':40,'sha256':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()}},indent=2)+'\n').encode()
    return files
if __name__=='__main__':
    OUT.mkdir(parents=True,exist_ok=True)
    for n,b in generate().items():(OUT/n).write_bytes(b)
