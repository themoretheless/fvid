#!/usr/bin/env python3
"""Offline authored MP4 companions for seek/resume device-clock ordering."""
import hashlib
import json
from pathlib import Path
from generate_mp4_audio_partial_window import generate as donors
OUT = Path(__file__).resolve().parents[1] / 'tests/fixtures/playback-errors/audio-seek-resume'
def generate():
    source = donors()
    files = {}
    cases = []
    for codec, frames in [('ima', 192), ('alac', 480)]:
        movie = f'{codec}.mp4'
        pcm = f'{codec}.f32le'
        files[movie] = source[f'{codec}-control.mp4']
        files[pcm] = source[f'{codec}-control.f32le']
        cases.append({'movie': movie, 'pcm': pcm, 'rate': 8000, 'frames': frames, 'seek_sample': 64 if codec == 'ima' else 160})
    files['manifest.json'] = (json.dumps({'provenance': 'Only committed authored neutral VP9 and analytic IMA4/authored ALAC PCM donors. Commands vary, not private media. Three video pictures accompany each audio track.', 'cases': cases, 'sha256': {n: hashlib.sha256(b).hexdigest() for n, b in files.items()}}, indent=2) + '\n').encode()
    return files
if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (OUT / name).write_bytes(data)
