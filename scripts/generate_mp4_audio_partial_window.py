#!/usr/bin/env python3
"""Authored positive terminal windows shorter than a coded audio block.

Uses only committed synthetic donors. Generation is separate from offline tests.
"""
import hashlib
import json
from pathlib import Path
from generate_mp4_audio_zero_window import rewrite
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'tests/fixtures/playback-errors/mp4-audio-partial-window'

def generate():
    donor = ROOT / 'tests/fixtures/playback-errors/mp4-audio-zero-window'
    files = {}
    cases = []
    for codec, block, tail in [('ima', 64, 16), ('alac', 160, 40)]:
        control = (donor / f'{codec}-control.mp4').read_bytes()
        pcm = (donor / f'{codec}-full.f32le').read_bytes()
        for label, last in [('control', block), ('partial', tail)]:
            name = f'{codec}-{label}'
            frames = block * 2 + last
            files[name + '.mp4'] = control if label == 'control' else rewrite(control, [(2, block), (1, last)], 8000, frames)
            files[name + '.f32le'] = pcm[:frames * 4]
            cases.append({'file': name + '.mp4', 'oracle': name + '.f32le', 'durations': [block, block, last], 'frames': frames, 'rate': 8000})
    files['manifest.json'] = (json.dumps({'provenance': 'Committed authored neutral VP9 with analytic IMA4 and authored ALAC scalar PCM. Only audio timing/header duration changes; coded packets unchanged.', 'cases': cases, 'sha256': {n: hashlib.sha256(b).hexdigest() for n,b in files.items()}}, indent=2) + '\n').encode()
    return files

if __name__ == '__main__':
    OUT.mkdir(parents=True, exist_ok=True)
    for name, body in generate().items():
        (OUT / name).write_bytes(body)
